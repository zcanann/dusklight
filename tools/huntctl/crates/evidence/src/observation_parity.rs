//! Cold replay proof that native learning-observation capture is gameplay inert.

use crate::artifact::Digest;
use crate::native_episode_shard::{NativeEpisodeShard, NativeRawPad};
use crate::trace::{DecodedTrace, TraceAppliedPads, TraceChannel, TraceRecord};
use dusklight_automation_contracts::tape::RawPadState;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;

pub const OBSERVATION_PARITY_REPORT_SCHEMA_V1: &str = "dusklight-observation-parity-report/v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationParityDivergence {
    pub domain: String,
    pub boundary_index: u64,
    pub detail: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationParityReport {
    pub schema: String,
    pub observation_off_trace_sha256: Digest,
    pub observation_on_trace_sha256: Digest,
    pub learning_shard_sha256: Digest,
    pub trace_version: u16,
    pub requested_channels: Vec<String>,
    pub source_frame: u64,
    pub compared_boundaries: u64,
    pub compared_learning_steps: u64,
    pub observation_off_raw_pad_series_sha256: Digest,
    pub observation_on_raw_pad_series_sha256: Digest,
    pub learning_consumed_pad_series_sha256: Digest,
    pub observation_on_suffix_pad_series_sha256: Digest,
    pub observation_off_state_series_sha256: Digest,
    pub observation_on_state_series_sha256: Digest,
    pub verified: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_divergence: Option<ObservationParityDivergence>,
    pub report_sha256: Digest,
}

impl ObservationParityReport {
    pub fn build(
        observation_off: &DecodedTrace,
        observation_off_bytes: &[u8],
        observation_on: &DecodedTrace,
        observation_on_bytes: &[u8],
        learning_shard: &NativeEpisodeShard,
    ) -> Result<Self, ObservationParityError> {
        validate_trace_pair(observation_off, observation_on)?;
        if learning_shard.episodes.len() != 1 || learning_shard.episodes[0].steps.is_empty() {
            return Err(ObservationParityError::new(
                "observation-on proof requires exactly one nonempty native learning episode",
            ));
        }

        let off_pads = raw_pad_series(observation_off)?;
        let on_pads = raw_pad_series(observation_on)?;
        let off_states = state_series(observation_off)?;
        let on_states = state_series(observation_on)?;
        let episode = &learning_shard.episodes[0];
        let learning_pads = episode
            .steps
            .iter()
            .map(|step| native_pad_to_contract(step.consumed_pad))
            .collect::<Vec<_>>();
        let on_suffix_pads = episode
            .steps
            .iter()
            .map(|step| {
                // The episode boundary is authoritative. A source checkpoint may
                // describe the state before the next applied tape frame, so
                // source_frame + step_index is not generally the consumed PAD's
                // frame identity.
                let tape_frame = step.post_simulation.tape_frame;
                let record = observation_on
                    .records
                    .iter()
                    .find(|record| record.tape_frame == Some(tape_frame))
                    .ok_or_else(|| {
                        ObservationParityError::new(format!(
                            "observation-on trace lacks learning tape frame {tape_frame}"
                        ))
                    })?;
                trace_port_zero(record)
            })
            .collect::<Result<Vec<_>, _>>()?;

        let mut first_divergence = trace_divergence(
            observation_off,
            observation_on,
            &off_pads,
            &on_pads,
            &off_states,
            &on_states,
        );
        if first_divergence.is_none() {
            for (index, step) in episode.steps.iter().enumerate() {
                let boundary_index = step.post_simulation.boundary_index;
                if step.chosen_pad != step.consumed_pad {
                    first_divergence = Some(ObservationParityDivergence {
                        domain: "learning_chosen_vs_consumed_pad".into(),
                        boundary_index,
                        detail: format!("learning step {index} changed the selected PAD"),
                    });
                    break;
                }
                if learning_pads[index] != on_suffix_pads[index] {
                    first_divergence = Some(ObservationParityDivergence {
                        domain: "learning_shard_vs_trace_pad".into(),
                        boundary_index,
                        detail: format!(
                            "learning step {index} consumed {:?}, trace frame {} applied {:?}",
                            learning_pads[index],
                            step.post_simulation.tape_frame,
                            on_suffix_pads[index],
                        ),
                    });
                    break;
                }
            }
        }

        let requested_channels = TraceChannel::ALL
            .into_iter()
            .map(|channel| channel.name().to_string())
            .collect();
        let mut report = Self {
            schema: OBSERVATION_PARITY_REPORT_SCHEMA_V1.into(),
            observation_off_trace_sha256: sha256(observation_off_bytes),
            observation_on_trace_sha256: sha256(observation_on_bytes),
            learning_shard_sha256: learning_shard.content_sha256,
            trace_version: observation_off.version,
            requested_channels,
            source_frame: learning_shard.source_frame,
            compared_boundaries: observation_off.records.len() as u64,
            compared_learning_steps: episode.steps.len() as u64,
            observation_off_raw_pad_series_sha256: canonical_digest(
                b"dusklight.observation-parity.raw-pad-series/v1\0",
                &off_pads,
            )?,
            observation_on_raw_pad_series_sha256: canonical_digest(
                b"dusklight.observation-parity.raw-pad-series/v1\0",
                &on_pads,
            )?,
            learning_consumed_pad_series_sha256: canonical_digest(
                b"dusklight.observation-parity.raw-pad-series/v1\0",
                &learning_pads,
            )?,
            observation_on_suffix_pad_series_sha256: canonical_digest(
                b"dusklight.observation-parity.raw-pad-series/v1\0",
                &on_suffix_pads,
            )?,
            observation_off_state_series_sha256: canonical_digest(
                b"dusklight.observation-parity.gameplay-state-series/v1\0",
                &off_states,
            )?,
            observation_on_state_series_sha256: canonical_digest(
                b"dusklight.observation-parity.gameplay-state-series/v1\0",
                &on_states,
            )?,
            verified: first_divergence.is_none(),
            first_divergence,
            report_sha256: Digest::ZERO,
        };
        report.report_sha256 = report.compute_identity()?;
        report.validate()?;
        Ok(report)
    }

    pub fn validate(&self) -> Result<(), ObservationParityError> {
        let complete_channels = TraceChannel::ALL
            .into_iter()
            .map(|channel| channel.name().to_string())
            .collect::<Vec<_>>();
        if self.schema != OBSERVATION_PARITY_REPORT_SCHEMA_V1
            || self.observation_off_trace_sha256 == Digest::ZERO
            || self.observation_on_trace_sha256 == Digest::ZERO
            || self.learning_shard_sha256 == Digest::ZERO
            || self.trace_version < 5
            || self.requested_channels != complete_channels
            || self.compared_boundaries == 0
            || self.compared_learning_steps == 0
            || self.verified != self.first_divergence.is_none()
            || self.report_sha256 != self.compute_identity()?
        {
            return Err(ObservationParityError::new(
                "observation parity report envelope or seal is invalid",
            ));
        }
        if self.verified
            && (self.observation_off_raw_pad_series_sha256
                != self.observation_on_raw_pad_series_sha256
                || self.learning_consumed_pad_series_sha256
                    != self.observation_on_suffix_pad_series_sha256
                || self.observation_off_state_series_sha256
                    != self.observation_on_state_series_sha256)
        {
            return Err(ObservationParityError::new(
                "verified observation parity report contains unequal evidence digests",
            ));
        }
        Ok(())
    }

    fn compute_identity(&self) -> Result<Digest, ObservationParityError> {
        let mut canonical = self.clone();
        canonical.report_sha256 = Digest::ZERO;
        canonical_digest(b"dusklight.observation-parity-report/v1\0", &canonical)
    }
}

fn validate_trace_pair(
    observation_off: &DecodedTrace,
    observation_on: &DecodedTrace,
) -> Result<(), ObservationParityError> {
    let all_channels = TraceChannel::ALL
        .into_iter()
        .fold(0_u64, |mask, channel| mask | channel.bit());
    if observation_off.version < 5
        || observation_on.version != observation_off.version
        || observation_off.boot != observation_on.boot
        || observation_off.tick_rate_numerator != observation_on.tick_rate_numerator
        || observation_off.tick_rate_denominator != observation_on.tick_rate_denominator
    {
        return Err(ObservationParityError::new(
            "cold traces use incompatible versions, boot origins, or tick rates",
        ));
    }
    if observation_off.requested_channels != all_channels
        || observation_on.requested_channels != all_channels
    {
        return Err(ObservationParityError::new(
            "observation parity requires every gameplay-trace channel in both cold runs",
        ));
    }
    if observation_off.capacity_exhausted
        || observation_on.capacity_exhausted
        || observation_off.retention.is_some()
        || observation_on.retention.is_some()
        || observation_off.records.is_empty()
        || observation_on.records.is_empty()
    {
        return Err(ObservationParityError::new(
            "observation parity requires complete, unretained, nonempty cold traces",
        ));
    }
    Ok(())
}

fn trace_divergence(
    observation_off: &DecodedTrace,
    observation_on: &DecodedTrace,
    off_pads: &[TraceAppliedPads],
    on_pads: &[TraceAppliedPads],
    off_states: &[Value],
    on_states: &[Value],
) -> Option<ObservationParityDivergence> {
    let count = observation_off
        .records
        .len()
        .max(observation_on.records.len());
    for index in 0..count {
        let off = observation_off.records.get(index);
        let on = observation_on.records.get(index);
        let boundary_index = off
            .or(on)
            .map_or(index as u64, |record| record.boundary_index);
        let same_boundary = off.zip(on).is_some_and(|(left, right)| {
            left.boundary_index == right.boundary_index
                && left.simulation_tick == right.simulation_tick
                && left.tape_frame == right.tape_frame
                && left.observation_phase == right.observation_phase
        });
        if !same_boundary {
            return Some(ObservationParityDivergence {
                domain: "boundary".into(),
                boundary_index,
                detail: "cold traces have different boundary identity or length".into(),
            });
        }
        if off_pads.get(index) != on_pads.get(index) {
            return Some(ObservationParityDivergence {
                domain: "raw_pad".into(),
                boundary_index,
                detail: "cold traces consumed different exact multi-port PAD state".into(),
            });
        }
        if off_states.get(index) != on_states.get(index) {
            return Some(ObservationParityDivergence {
                domain: "gameplay_state".into(),
                boundary_index,
                detail: "cold traces produced different full-channel gameplay state".into(),
            });
        }
    }
    None
}

fn raw_pad_series(trace: &DecodedTrace) -> Result<Vec<TraceAppliedPads>, ObservationParityError> {
    trace
        .records
        .iter()
        .map(|record| {
            record.applied_pads.clone().ok_or_else(|| {
                ObservationParityError::new(format!(
                    "boundary {} has no applied-PAD observation",
                    record.boundary_index
                ))
            })
        })
        .collect()
}

fn trace_port_zero(record: &TraceRecord) -> Result<RawPadState, ObservationParityError> {
    let pads = record.applied_pads.as_ref().ok_or_else(|| {
        ObservationParityError::new(format!(
            "boundary {} has no applied-PAD observation",
            record.boundary_index
        ))
    })?;
    if pads.valid_ports & 1 == 0 || pads.owned_ports & 1 == 0 {
        return Err(ObservationParityError::new(format!(
            "boundary {} does not contain tape-owned port-zero PAD",
            record.boundary_index
        )));
    }
    Ok(pads.pads[0])
}

fn state_series(trace: &DecodedTrace) -> Result<Vec<Value>, ObservationParityError> {
    trace
        .records
        .iter()
        .map(|record| {
            let mut value = serde_json::to_value(record)
                .map_err(|error| ObservationParityError::new(error.to_string()))?;
            let object = value.as_object_mut().ok_or_else(|| {
                ObservationParityError::new("serialized trace record is not an object")
            })?;
            for input_field in [
                "input_source",
                "buttons",
                "stick_x",
                "stick_y",
                "pad_error",
                "applied_pads",
            ] {
                object.remove(input_field);
            }
            Ok(value)
        })
        .collect()
}

fn native_pad_to_contract(pad: NativeRawPad) -> RawPadState {
    RawPadState {
        buttons: pad.buttons,
        stick_x: pad.stick_x,
        stick_y: pad.stick_y,
        substick_x: pad.substick_x,
        substick_y: pad.substick_y,
        trigger_left: pad.trigger_left,
        trigger_right: pad.trigger_right,
        analog_a: pad.analog_a,
        analog_b: pad.analog_b,
        connected: pad.connected,
        error: pad.error,
    }
}

fn sha256(bytes: &[u8]) -> Digest {
    Digest(Sha256::digest(bytes).into())
}

fn canonical_digest(
    domain: &[u8],
    value: &impl Serialize,
) -> Result<Digest, ObservationParityError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| ObservationParityError::new(error.to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
    Ok(Digest(hasher.finalize().into()))
}

#[derive(Debug)]
pub struct ObservationParityError(String);

impl ObservationParityError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for ObservationParityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for ObservationParityError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_episode_shard::NativeEpisodeShard;
    use crate::trace::{TraceAppliedPads, TraceChannelStatus, TracePhase};
    use dusklight_automation_contracts::tape::TapeBoot;
    use std::collections::BTreeMap;

    fn golden_v4() -> NativeEpisodeShard {
        let mut shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v4.dseps"
        ))
        .unwrap();
        shard.episodes.truncate(1);
        shard
    }

    fn trace_for(shard: &NativeEpisodeShard) -> DecodedTrace {
        let pad = native_pad_to_contract(shard.episodes[0].steps[0].consumed_pad);
        let channel_status = TraceChannel::ALL
            .into_iter()
            .map(|channel| (channel, TraceChannelStatus::Present))
            .collect::<BTreeMap<_, _>>();
        let post = &shard.episodes[0].steps[0].post_simulation;
        DecodedTrace {
            version: 5,
            boot: TapeBoot::Process,
            tick_rate_numerator: 30,
            tick_rate_denominator: 1,
            requested_channels: TraceChannel::ALL
                .into_iter()
                .fold(0, |mask, channel| mask | channel.bit()),
            capacity_exhausted: false,
            retention: None,
            channel_formats: BTreeMap::new(),
            records: vec![TraceRecord {
                boundary_index: post.boundary_index,
                simulation_tick: post.simulation_tick,
                tape_frame: Some(shard.source_frame),
                observation_phase: TracePhase::PostSimulation,
                input_source: 1,
                channel_status,
                applied_pads: Some(TraceAppliedPads {
                    valid_ports: 1,
                    owned_ports: 1,
                    pads: [
                        pad,
                        RawPadState::default(),
                        RawPadState::default(),
                        RawPadState::default(),
                    ],
                }),
                ..TraceRecord::default()
            }],
        }
    }

    #[test]
    fn seals_matching_cold_traces_and_native_learning_pad() {
        let shard = golden_v4();
        let trace = trace_for(&shard);
        let report = ObservationParityReport::build(&trace, b"off", &trace, b"on", &shard)
            .expect("matching evidence builds");
        assert!(report.verified);
        assert_eq!(report.compared_boundaries, 1);
        assert_eq!(report.compared_learning_steps, 1);
        report.validate().unwrap();
    }

    #[test]
    fn reports_state_and_pad_divergence() {
        let shard = golden_v4();
        let trace = trace_for(&shard);
        let mut changed = trace.clone();
        changed.records[0].position[0] = 99.0;
        let report = ObservationParityReport::build(&trace, b"off", &changed, b"on", &shard)
            .expect("mismatch remains inspectable");
        assert!(!report.verified);
        assert_eq!(report.first_divergence.unwrap().domain, "gameplay_state");

        let mut changed = trace.clone();
        changed.records[0].applied_pads.as_mut().unwrap().pads[2].buttons ^= 1;
        let report = ObservationParityReport::build(&trace, b"off", &changed, b"on", &shard)
            .expect("mismatch remains inspectable");
        assert_eq!(report.first_divergence.unwrap().domain, "raw_pad");
    }

    #[test]
    fn rejects_partial_or_retained_traces() {
        let shard = golden_v4();
        let mut trace = trace_for(&shard);
        trace.requested_channels = TraceChannel::Core.bit();
        assert!(ObservationParityReport::build(&trace, b"off", &trace, b"on", &shard).is_err());
    }
}
