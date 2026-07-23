//! Sealed per-boundary evidence for return/restart writer execution and value change.

use crate::artifact::Digest;
use crate::native_episode_shard::{
    LEARNING_OBSERVATION_SCHEMA_V28, NativeChannelStatus, NativeEpisodeShard,
    NativeLearningObservation, NativeRestartObservation, NativeReturnPlaceObservation,
    NativeReturnRestartWriteTraceObservation,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;

pub const NATIVE_RETURN_RESTART_WRITE_TRACE_SCHEMA_V1: &str =
    "dusklight-native-return-restart-write-trace/v1";
const MAX_SOURCE_SHARDS: usize = 4096;
const MAX_EVENTS: usize = 10_000_000;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeReturnRestartWriteTrace {
    pub schema: String,
    pub source_shards: Vec<ReturnRestartTraceSource>,
    pub summary: ReturnRestartTraceSummary,
    pub events: Vec<ReturnRestartTraceEvent>,
    pub content_sha256: Digest,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReturnRestartTraceSource {
    pub shard_sha256: Digest,
    pub observation_schema: String,
    pub checkpoint_identity: String,
    pub objective: String,
    pub episodes: u64,
    pub observations: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReturnRestartTraceSummary {
    pub observations: u64,
    pub event_boundaries: u64,
    pub return_place_initialize_writes: u64,
    pub return_place_set_writes: u64,
    pub savmem_executes: u64,
    pub savmem_eligible_executes: u64,
    pub restart_place_writes: u64,
    pub restart_start_point_writes: u64,
    pub restart_room_parameter_writes: u64,
    pub restart_last_scene_info_writes: u64,
    pub return_place_value_changes: u64,
    pub restart_place_value_changes: u64,
    pub restart_start_point_value_changes: u64,
    pub restart_room_parameter_value_changes: u64,
    pub restart_last_scene_info_value_changes: u64,
    pub savmem_eligible_without_value_change_boundaries: u64,
    pub return_writes_without_value_change_boundaries: u64,
    pub restart_writes_without_value_change_boundaries: u64,
    pub restart_start_point_writes_without_value_change_boundaries: u64,
    pub restart_parameter_writes_without_value_change_boundaries: u64,
    pub restart_last_scene_info_writes_without_value_change_boundaries: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReturnRestartTraceEvent {
    pub source_shard_sha256: Digest,
    pub episode_id: String,
    pub phase: ReturnRestartTracePhase,
    pub boundary_index: u64,
    pub simulation_tick: u64,
    pub tape_frame: Option<u64>,
    pub stage: String,
    pub room: i8,
    pub layer: i8,
    pub writes: ReturnRestartWriteCounts,
    pub return_place_before: Option<ReturnPlaceTraceValue>,
    pub return_place_after: Option<ReturnPlaceTraceValue>,
    pub return_place_net_changed: Option<bool>,
    pub restart_before: Option<RestartTraceValue>,
    pub restart_after: Option<RestartTraceValue>,
    pub restart_net_changed: Option<bool>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReturnRestartTracePhase {
    PreInput,
    PostSimulation,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReturnRestartWriteCounts {
    pub return_place_initialize_count: u16,
    pub return_place_set_count: u16,
    pub savmem_execute_count: u16,
    pub savmem_eligible_execute_count: u16,
    pub restart_place_set_count: u16,
    pub restart_start_point_set_count: u16,
    pub restart_room_parameter_set_count: u16,
    pub restart_last_scene_info_set_count: u16,
    pub return_place_value_change_count: u16,
    pub restart_place_value_change_count: u16,
    pub restart_start_point_value_change_count: u16,
    pub restart_room_parameter_value_change_count: u16,
    pub restart_last_scene_info_value_change_count: u16,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReturnPlaceTraceValue {
    pub stage: String,
    pub room: i8,
    pub player_status: u8,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RestartTraceValue {
    pub room: i8,
    pub start_point: i16,
    pub angle_y: i16,
    pub position: [f32; 3],
    pub room_param: u32,
    pub last_speed: f32,
    pub last_mode: u32,
    pub last_angle_y: i16,
}

impl NativeReturnRestartWriteTrace {
    pub fn build(shards: &[NativeEpisodeShard]) -> Result<Self, NativeReturnRestartTraceError> {
        if shards.is_empty() || shards.len() > MAX_SOURCE_SHARDS {
            return Err(trace_error(format!(
                "return/restart trace requires 1..={MAX_SOURCE_SHARDS} source shards"
            )));
        }
        let mut ordered = shards.iter().collect::<Vec<_>>();
        ordered.sort_by_key(|shard| shard.content_sha256);
        if ordered
            .windows(2)
            .any(|pair| pair[0].content_sha256 == pair[1].content_sha256)
        {
            return Err(trace_error(
                "return/restart trace source shards are duplicated",
            ));
        }
        let mut sources = Vec::with_capacity(ordered.len());
        let mut events = Vec::new();
        let mut summary = ReturnRestartTraceSummary::default();
        for shard in ordered {
            if shard.metadata.observation_schema != LEARNING_OBSERVATION_SCHEMA_V28 {
                return Err(trace_error(
                    "return/restart trace requires v28 write telemetry",
                ));
            }
            let observation_count = shard
                .episodes
                .iter()
                .try_fold(0_u64, |count, episode| {
                    count.checked_add((episode.steps.len() as u64).saturating_mul(2))
                })
                .ok_or_else(|| trace_error("return/restart observation count overflowed"))?;
            summary.observations = summary
                .observations
                .checked_add(observation_count)
                .ok_or_else(|| trace_error("return/restart observation count overflowed"))?;
            sources.push(ReturnRestartTraceSource {
                shard_sha256: shard.content_sha256,
                observation_schema: shard.metadata.observation_schema.clone(),
                checkpoint_identity: shard.metadata.checkpoint_identity.clone(),
                objective: shard.metadata.objective.clone(),
                episodes: shard.episodes.len() as u64,
                observations: observation_count,
            });
            for episode in &shard.episodes {
                let mut previous: Option<&NativeLearningObservation> = None;
                for step in &episode.steps {
                    for (phase, observation) in [
                        (ReturnRestartTracePhase::PreInput, &step.pre_input),
                        (
                            ReturnRestartTracePhase::PostSimulation,
                            &step.post_simulation,
                        ),
                    ] {
                        if observation.return_restart_write_trace_status
                            != NativeChannelStatus::Present
                        {
                            return Err(trace_error(
                                "v28 observation has unavailable return/restart write telemetry",
                            ));
                        }
                        if let Some(writes) = observation.return_restart_write_trace {
                            if !valid_counts(writes) {
                                return Err(trace_error(
                                    "v28 observation has inconsistent return/restart write telemetry",
                                ));
                            }
                            accumulate(&mut summary, writes)?;
                            if has_event(writes) {
                                if events.len() == MAX_EVENTS {
                                    return Err(trace_error(format!(
                                        "return/restart trace exceeds {MAX_EVENTS} events"
                                    )));
                                }
                                events.push(make_event(
                                    shard.content_sha256,
                                    &episode.id,
                                    phase,
                                    previous,
                                    observation,
                                    writes,
                                ));
                            }
                        } else {
                            return Err(trace_error(
                                "v28 observation is missing return/restart write telemetry",
                            ));
                        }
                        previous = Some(observation);
                    }
                }
            }
        }
        events.sort_by(|left, right| event_key(left).cmp(&event_key(right)));
        summary.event_boundaries = events.len() as u64;
        let mut report = Self {
            schema: NATIVE_RETURN_RESTART_WRITE_TRACE_SCHEMA_V1.into(),
            source_shards: sources,
            summary,
            events,
            content_sha256: Digest::ZERO,
        };
        report.content_sha256 = report.compute_identity()?;
        report.validate()?;
        Ok(report)
    }

    pub fn validate(&self) -> Result<(), NativeReturnRestartTraceError> {
        if self.schema != NATIVE_RETURN_RESTART_WRITE_TRACE_SCHEMA_V1
            || self.source_shards.is_empty()
            || self.source_shards.len() > MAX_SOURCE_SHARDS
            || self.source_shards.windows(2).any(|pair| pair[0] >= pair[1])
            || self.source_shards.iter().any(|source| {
                source.shard_sha256 == Digest::ZERO
                    || source.observation_schema != LEARNING_OBSERVATION_SCHEMA_V28
                    || source.checkpoint_identity.is_empty()
                    || source.objective.is_empty()
                    || source.episodes == 0
                    || source.observations == 0
            })
            || self.events.len() > MAX_EVENTS
            || self.content_sha256 == Digest::ZERO
            || self.content_sha256 != self.compute_identity()?
        {
            return Err(trace_error(
                "return/restart trace envelope, ordering, or seal is invalid",
            ));
        }
        let source_digests = self
            .source_shards
            .iter()
            .map(|source| source.shard_sha256)
            .collect::<Vec<_>>();
        if self
            .events
            .windows(2)
            .any(|pair| event_key(&pair[0]) >= event_key(&pair[1]))
            || self.events.iter().any(|event| {
                source_digests
                    .binary_search(&event.source_shard_sha256)
                    .is_err()
                    || event.episode_id.is_empty()
                    || event.stage.is_empty()
                    || !has_event(event.writes.into())
                    || !valid_counts(event.writes.into())
                    || !event
                        .restart_before
                        .iter()
                        .chain(event.restart_after.iter())
                        .all(valid_restart)
                    || event.return_place_net_changed
                        != comparable_change(&event.return_place_before, &event.return_place_after)
                    || event.restart_net_changed
                        != comparable_change(&event.restart_before, &event.restart_after)
            })
        {
            return Err(trace_error(
                "return/restart trace contains invalid or unordered events",
            ));
        }
        let mut summary = ReturnRestartTraceSummary::default();
        summary.observations = self
            .source_shards
            .iter()
            .try_fold(0_u64, |count, source| {
                count.checked_add(source.observations)
            })
            .ok_or_else(|| trace_error("return/restart summary overflowed"))?;
        for event in &self.events {
            accumulate(&mut summary, event.writes.into())?;
        }
        summary.event_boundaries = self.events.len() as u64;
        if summary != self.summary {
            return Err(trace_error(
                "return/restart trace summary does not reproduce its events",
            ));
        }
        Ok(())
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, NativeReturnRestartTraceError> {
        let report: Self = serde_json::from_slice(bytes).map_err(trace_error)?;
        report.validate()?;
        Ok(report)
    }

    fn compute_identity(&self) -> Result<Digest, NativeReturnRestartTraceError> {
        let mut canonical = self.clone();
        canonical.content_sha256 = Digest::ZERO;
        let bytes = serde_json::to_vec(&canonical).map_err(trace_error)?;
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight.native-return-restart-write-trace/v1\0");
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
        Ok(Digest(hasher.finalize().into()))
    }
}

impl From<NativeReturnRestartWriteTraceObservation> for ReturnRestartWriteCounts {
    fn from(value: NativeReturnRestartWriteTraceObservation) -> Self {
        Self {
            return_place_initialize_count: value.return_place_initialize_count,
            return_place_set_count: value.return_place_set_count,
            savmem_execute_count: value.savmem_execute_count,
            savmem_eligible_execute_count: value.savmem_eligible_execute_count,
            restart_place_set_count: value.restart_place_set_count,
            restart_start_point_set_count: value.restart_start_point_set_count,
            restart_room_parameter_set_count: value.restart_room_parameter_set_count,
            restart_last_scene_info_set_count: value.restart_last_scene_info_set_count,
            return_place_value_change_count: value.return_place_value_change_count,
            restart_place_value_change_count: value.restart_place_value_change_count,
            restart_start_point_value_change_count: value.restart_start_point_value_change_count,
            restart_room_parameter_value_change_count: value
                .restart_room_parameter_value_change_count,
            restart_last_scene_info_value_change_count: value
                .restart_last_scene_info_value_change_count,
        }
    }
}

impl From<ReturnRestartWriteCounts> for NativeReturnRestartWriteTraceObservation {
    fn from(value: ReturnRestartWriteCounts) -> Self {
        Self {
            return_place_initialize_count: value.return_place_initialize_count,
            return_place_set_count: value.return_place_set_count,
            savmem_execute_count: value.savmem_execute_count,
            savmem_eligible_execute_count: value.savmem_eligible_execute_count,
            restart_place_set_count: value.restart_place_set_count,
            restart_start_point_set_count: value.restart_start_point_set_count,
            restart_room_parameter_set_count: value.restart_room_parameter_set_count,
            restart_last_scene_info_set_count: value.restart_last_scene_info_set_count,
            return_place_value_change_count: value.return_place_value_change_count,
            restart_place_value_change_count: value.restart_place_value_change_count,
            restart_start_point_value_change_count: value.restart_start_point_value_change_count,
            restart_room_parameter_value_change_count: value
                .restart_room_parameter_value_change_count,
            restart_last_scene_info_value_change_count: value
                .restart_last_scene_info_value_change_count,
        }
    }
}

fn make_event(
    source_shard_sha256: Digest,
    episode_id: &str,
    phase: ReturnRestartTracePhase,
    previous: Option<&NativeLearningObservation>,
    observation: &NativeLearningObservation,
    writes: NativeReturnRestartWriteTraceObservation,
) -> ReturnRestartTraceEvent {
    let return_place_before = previous
        .and_then(|value| value.return_place.as_ref())
        .map(Into::into);
    let return_place_after = observation.return_place.as_ref().map(Into::into);
    let restart_before = previous
        .and_then(|value| value.restart.as_ref())
        .map(Into::into);
    let restart_after = observation.restart.as_ref().map(Into::into);
    ReturnRestartTraceEvent {
        source_shard_sha256,
        episode_id: episode_id.into(),
        phase,
        boundary_index: observation.boundary_index,
        simulation_tick: observation.simulation_tick,
        tape_frame: (observation.tape_frame != u64::MAX).then_some(observation.tape_frame),
        stage: observation.stage.clone(),
        room: observation.room,
        layer: observation.layer,
        writes: writes.into(),
        return_place_net_changed: comparable_change(&return_place_before, &return_place_after),
        return_place_before,
        return_place_after,
        restart_net_changed: comparable_change(&restart_before, &restart_after),
        restart_before,
        restart_after,
    }
}

impl From<&NativeReturnPlaceObservation> for ReturnPlaceTraceValue {
    fn from(value: &NativeReturnPlaceObservation) -> Self {
        Self {
            stage: value.stage.clone(),
            room: value.room,
            player_status: value.player_status,
        }
    }
}

impl From<&NativeRestartObservation> for RestartTraceValue {
    fn from(value: &NativeRestartObservation) -> Self {
        Self {
            room: value.room,
            start_point: value.start_point,
            angle_y: value.angle_y,
            position: value.position,
            room_param: value.room_param,
            last_speed: value.last_speed,
            last_mode: value.last_mode,
            last_angle_y: value.last_angle_y,
        }
    }
}

fn has_event(writes: NativeReturnRestartWriteTraceObservation) -> bool {
    writes.return_place_initialize_count != 0
        || writes.return_place_set_count != 0
        || writes.savmem_execute_count != 0
        || writes.restart_place_set_count != 0
        || writes.restart_start_point_set_count != 0
        || writes.restart_room_parameter_set_count != 0
        || writes.restart_last_scene_info_set_count != 0
}

fn valid_counts(writes: NativeReturnRestartWriteTraceObservation) -> bool {
    let return_writes =
        u32::from(writes.return_place_initialize_count) + u32::from(writes.return_place_set_count);
    writes.savmem_eligible_execute_count <= writes.savmem_execute_count
        && u32::from(writes.return_place_value_change_count) <= return_writes
        && writes.restart_place_value_change_count <= writes.restart_place_set_count
        && writes.restart_start_point_value_change_count <= writes.restart_start_point_set_count
        && writes.restart_room_parameter_value_change_count
            <= writes.restart_room_parameter_set_count
        && writes.restart_last_scene_info_value_change_count
            <= writes.restart_last_scene_info_set_count
}

fn accumulate(
    summary: &mut ReturnRestartTraceSummary,
    writes: NativeReturnRestartWriteTraceObservation,
) -> Result<(), NativeReturnRestartTraceError> {
    let add = |target: &mut u64, value: u16| -> Result<(), NativeReturnRestartTraceError> {
        *target = target
            .checked_add(u64::from(value))
            .ok_or_else(|| trace_error("return/restart summary overflowed"))?;
        Ok(())
    };
    add(
        &mut summary.return_place_initialize_writes,
        writes.return_place_initialize_count,
    )?;
    add(
        &mut summary.return_place_set_writes,
        writes.return_place_set_count,
    )?;
    add(&mut summary.savmem_executes, writes.savmem_execute_count)?;
    add(
        &mut summary.savmem_eligible_executes,
        writes.savmem_eligible_execute_count,
    )?;
    add(
        &mut summary.restart_place_writes,
        writes.restart_place_set_count,
    )?;
    add(
        &mut summary.restart_start_point_writes,
        writes.restart_start_point_set_count,
    )?;
    add(
        &mut summary.restart_room_parameter_writes,
        writes.restart_room_parameter_set_count,
    )?;
    add(
        &mut summary.restart_last_scene_info_writes,
        writes.restart_last_scene_info_set_count,
    )?;
    add(
        &mut summary.return_place_value_changes,
        writes.return_place_value_change_count,
    )?;
    add(
        &mut summary.restart_place_value_changes,
        writes.restart_place_value_change_count,
    )?;
    add(
        &mut summary.restart_start_point_value_changes,
        writes.restart_start_point_value_change_count,
    )?;
    add(
        &mut summary.restart_room_parameter_value_changes,
        writes.restart_room_parameter_value_change_count,
    )?;
    add(
        &mut summary.restart_last_scene_info_value_changes,
        writes.restart_last_scene_info_value_change_count,
    )?;
    summary.savmem_eligible_without_value_change_boundaries += u64::from(
        writes.savmem_eligible_execute_count != 0 && writes.return_place_value_change_count == 0,
    );
    summary.return_writes_without_value_change_boundaries += u64::from(
        (writes.return_place_initialize_count != 0 || writes.return_place_set_count != 0)
            && writes.return_place_value_change_count == 0,
    );
    summary.restart_writes_without_value_change_boundaries += u64::from(
        writes.restart_place_set_count != 0 && writes.restart_place_value_change_count == 0,
    );
    summary.restart_start_point_writes_without_value_change_boundaries += u64::from(
        writes.restart_start_point_set_count != 0
            && writes.restart_start_point_value_change_count == 0,
    );
    summary.restart_parameter_writes_without_value_change_boundaries += u64::from(
        writes.restart_room_parameter_set_count != 0
            && writes.restart_room_parameter_value_change_count == 0,
    );
    summary.restart_last_scene_info_writes_without_value_change_boundaries += u64::from(
        writes.restart_last_scene_info_set_count != 0
            && writes.restart_last_scene_info_value_change_count == 0,
    );
    Ok(())
}

fn comparable_change<T: PartialEq>(before: &Option<T>, after: &Option<T>) -> Option<bool> {
    before
        .as_ref()
        .zip(after.as_ref())
        .map(|(before, after)| before != after)
}

fn valid_restart(value: &RestartTraceValue) -> bool {
    value.position.iter().all(|component| component.is_finite()) && value.last_speed.is_finite()
}

fn event_key(event: &ReturnRestartTraceEvent) -> (Digest, &str, u64, ReturnRestartTracePhase) {
    (
        event.source_shard_sha256,
        &event.episode_id,
        event.boundary_index,
        event.phase,
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeReturnRestartTraceError(String);

impl fmt::Display for NativeReturnRestartTraceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for NativeReturnRestartTraceError {}

fn trace_error(error: impl ToString) -> NativeReturnRestartTraceError {
    NativeReturnRestartTraceError(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_episode_shard::{NativeChannelStatus, NativeEpisodeShard};

    fn traced_v28_shard() -> NativeEpisodeShard {
        let bytes =
            include_bytes!("../../../../../tests/fixtures/automation/native_episode_v28.dseps");
        let mut shard = NativeEpisodeShard::decode(bytes).unwrap();
        let zero = NativeReturnRestartWriteTraceObservation {
            return_place_initialize_count: 0,
            return_place_set_count: 0,
            savmem_execute_count: 0,
            savmem_eligible_execute_count: 0,
            restart_place_set_count: 0,
            restart_start_point_set_count: 0,
            restart_room_parameter_set_count: 0,
            restart_last_scene_info_set_count: 0,
            return_place_value_change_count: 0,
            restart_place_value_change_count: 0,
            restart_start_point_value_change_count: 0,
            restart_room_parameter_value_change_count: 0,
            restart_last_scene_info_value_change_count: 0,
        };
        for observation in shard.episodes.iter_mut().flat_map(|episode| {
            episode
                .steps
                .iter_mut()
                .flat_map(|step| [&mut step.pre_input, &mut step.post_simulation])
        }) {
            observation.return_restart_write_trace_status = NativeChannelStatus::Present;
            observation.return_restart_write_trace = Some(zero);
        }
        shard.episodes[0].steps[0]
            .post_simulation
            .return_restart_write_trace = Some(NativeReturnRestartWriteTraceObservation {
            return_place_initialize_count: 0,
            return_place_set_count: 1,
            savmem_execute_count: 1,
            savmem_eligible_execute_count: 1,
            restart_place_set_count: 0,
            restart_start_point_set_count: 0,
            restart_room_parameter_set_count: 0,
            restart_last_scene_info_set_count: 0,
            return_place_value_change_count: 0,
            restart_place_value_change_count: 0,
            restart_start_point_value_change_count: 0,
            restart_room_parameter_value_change_count: 0,
            restart_last_scene_info_value_change_count: 0,
        });
        shard
    }

    #[test]
    fn seals_idempotent_writer_execution_separately_from_value_change() {
        let report = NativeReturnRestartWriteTrace::build(&[traced_v28_shard()]).unwrap();
        assert_eq!(report.events.len(), 1);
        assert_eq!(report.summary.savmem_executes, 1);
        assert_eq!(report.summary.savmem_eligible_executes, 1);
        assert_eq!(report.summary.return_place_set_writes, 1);
        assert_eq!(report.summary.return_place_value_changes, 0);
        assert_eq!(
            report
                .summary
                .savmem_eligible_without_value_change_boundaries,
            1
        );
        assert_eq!(
            report.summary.return_writes_without_value_change_boundaries,
            1
        );
        assert_eq!(
            NativeReturnRestartWriteTrace::decode(&serde_json::to_vec(&report).unwrap()).unwrap(),
            report
        );
    }

    #[test]
    fn rejects_tampered_write_trace_summary() {
        let report = NativeReturnRestartWriteTrace::build(&[traced_v28_shard()]).unwrap();
        let mut value = serde_json::to_value(report).unwrap();
        value["summary"]["return_place_set_writes"] = serde_json::json!(2);
        assert!(
            NativeReturnRestartWriteTrace::decode(&serde_json::to_vec(&value).unwrap()).is_err()
        );
    }

    #[test]
    fn rejects_unavailable_or_inconsistent_source_telemetry() {
        let mut unavailable = traced_v28_shard();
        unavailable.episodes[0].steps[0]
            .pre_input
            .return_restart_write_trace_status = NativeChannelStatus::Unavailable;
        assert!(NativeReturnRestartWriteTrace::build(&[unavailable]).is_err());

        let mut inconsistent = traced_v28_shard();
        inconsistent.episodes[0].steps[0]
            .post_simulation
            .return_restart_write_trace
            .as_mut()
            .unwrap()
            .savmem_eligible_execute_count = 2;
        assert!(NativeReturnRestartWriteTrace::build(&[inconsistent]).is_err());
    }
}
