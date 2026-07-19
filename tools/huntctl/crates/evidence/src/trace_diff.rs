//! Typed success/failure sibling trace comparison.

use crate::artifact::Digest;
use crate::trace::{DecodedTrace, TraceChannel, TraceChannelStatus, TraceRecord};
use crate::transition_evidence::TransitionEvidenceBundle;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

pub const SIBLING_TRACE_DIFF_SCHEMA_V1: &str = "dusklight-sibling-trace-diff/v1";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffCoverage {
    Complete,
    Partial,
    Unavailable,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SiblingTraceDiff {
    pub schema: String,
    pub success_trace_sha256: Digest,
    pub failure_trace_sha256: Digest,
    pub compared_boundaries: u64,
    pub first_divergence_boundary: Option<u64>,
    pub domains: BTreeMap<String, DomainDiff>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DomainDiff {
    pub coverage: DiffCoverage,
    pub first_difference_boundary: Option<u64>,
    pub difference_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub success_value: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_value: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limitation: Option<String>,
}

impl SiblingTraceDiff {
    pub fn compare(
        success: &DecodedTrace,
        success_bytes: &[u8],
        failure: &DecodedTrace,
        failure_bytes: &[u8],
        success_evidence: Option<&TransitionEvidenceBundle>,
        failure_evidence: Option<&TransitionEvidenceBundle>,
    ) -> Result<Self, TraceDiffError> {
        if success.tick_rate_numerator != failure.tick_rate_numerator
            || success.tick_rate_denominator != failure.tick_rate_denominator
            || success.boot != failure.boot
        {
            return Err(TraceDiffError::new(
                "sibling traces use different boot origins or tick rates",
            ));
        }
        let success_records: BTreeMap<_, _> = success
            .records
            .iter()
            .map(|record| (record.boundary_index, record))
            .collect();
        let failure_records: BTreeMap<_, _> = failure
            .records
            .iter()
            .map(|record| (record.boundary_index, record))
            .collect();
        if success_records.len() != success.records.len()
            || failure_records.len() != failure.records.len()
        {
            return Err(TraceDiffError::new("trace repeats a boundary index"));
        }
        let boundaries: BTreeSet<_> = success_records
            .keys()
            .chain(failure_records.keys())
            .copied()
            .collect();
        let success_objective = objective_evidence_by_boundary(success_evidence)?;
        let failure_objective = objective_evidence_by_boundary(failure_evidence)?;
        let mut domains = BTreeMap::new();
        domains.insert(
            "phase".into(),
            compare_domain(
                &boundaries,
                &success_records,
                &failure_records,
                |record, _| {
                    record.map(|record| {
                        json!({
                            "phase": record.observation_phase,
                            "simulation_tick": record.simulation_tick,
                            "tape_frame": record.tape_frame,
                        })
                    })
                },
            ),
        );
        domains.insert(
            "event".into(),
            compare_domain(&boundaries, &success_records, &failure_records, |record, _| {
                available_channel(record, TraceChannel::Event).map(|record| {
                    json!({
                        "flags": record.flags,
                        "event_id": record.event_id,
                        "mode": record.event_mode,
                        "status": record.event_status,
                        "map_tool_id": record.event_map_tool_id,
                        "name_hash": record.event_name_hash_present.then_some(record.event_name_hash),
                    })
                })
            }),
        );
        domains.insert(
            "actor".into(),
            compare_domain(
                &boundaries,
                &success_records,
                &failure_records,
                |record, _| {
                    available_channel(record, TraceChannel::SelectedActors)
                        .and_then(|record| serde_json::to_value(&record.selected_actors).ok())
                },
            ),
        );
        domains.insert(
            "contact".into(),
            compare_domain(
                &boundaries,
                &success_records,
                &failure_records,
                |record, _| {
                    record.and_then(|record| {
                        (available_channel(Some(record), TraceChannel::PlayerBackgroundCollision)
                            .is_some()
                            || available_channel(
                                Some(record),
                                TraceChannel::PlayerCollisionSurfaces,
                            )
                            .is_some())
                        .then(|| {
                            json!({
                                "background": record.player_background_collision,
                                "surfaces": record.player_collision_surfaces,
                            })
                        })
                    })
                },
            ),
        );
        domains.insert(
            "flag".into(),
            compare_domain(
                &boundaries,
                &success_records,
                &failure_records,
                |record, _| record.map(|record| json!({ "core_flags": record.flags })),
            ),
        );
        domains.insert(
            "rng_draw".into(),
            compare_domain(
                &boundaries,
                &success_records,
                &failure_records,
                |record, _| {
                    available_channel(record, TraceChannel::Rng)
                        .and_then(|record| serde_json::to_value(&record.rng).ok())
                },
            ),
        );
        let mut allocation = compare_domain(
            &boundaries,
            &success_records,
            &failure_records,
            |record, _| {
                available_channel(record, TraceChannel::SelectedActors).and_then(|record| {
                    record.selected_actors.as_ref().map(|actors| {
                        json!({
                            "selected_actor_process_ids": actors.actors.iter()
                                .map(|actor| actor.session_process_id).collect::<Vec<_>>(),
                            "observed_count": actors.observed_count,
                            "truncated": actors.truncated,
                        })
                    })
                })
            },
        );
        if allocation.coverage != DiffCoverage::Unavailable {
            allocation.coverage = DiffCoverage::Partial;
            allocation.limitation = Some(
                "selected-actor process population only; heap/allocation events are not present in this trace schema"
                    .into(),
            );
        } else {
            allocation.limitation = Some(
                "heap/allocation and selected-actor population channels are unavailable".into(),
            );
        }
        domains.insert("allocation".into(), allocation);
        domains.insert(
            "objective_component".into(),
            compare_objective_domain(
                &boundaries,
                &success_records,
                &failure_records,
                &success_objective,
                &failure_objective,
            ),
        );
        let first_divergence_boundary = domains
            .values()
            .filter_map(|domain| domain.first_difference_boundary)
            .min();
        Ok(Self {
            schema: SIBLING_TRACE_DIFF_SCHEMA_V1.into(),
            success_trace_sha256: Digest(Sha256::digest(success_bytes).into()),
            failure_trace_sha256: Digest(Sha256::digest(failure_bytes).into()),
            compared_boundaries: boundaries.len() as u64,
            first_divergence_boundary,
            domains,
        })
    }
}

fn compare_domain(
    boundaries: &BTreeSet<u64>,
    success: &BTreeMap<u64, &TraceRecord>,
    failure: &BTreeMap<u64, &TraceRecord>,
    value: impl Fn(Option<&TraceRecord>, u64) -> Option<Value>,
) -> DomainDiff {
    let mut first = None;
    let mut difference_count = 0;
    let mut first_values = (None, None);
    let mut success_available = false;
    let mut failure_available = false;
    for boundary in boundaries {
        let left = value(success.get(boundary).copied(), *boundary);
        let right = value(failure.get(boundary).copied(), *boundary);
        success_available |= left.is_some();
        failure_available |= right.is_some();
        if left != right {
            difference_count += 1;
            if first.is_none() {
                first = Some(*boundary);
                first_values = (left, right);
            }
        }
    }
    DomainDiff {
        coverage: match (success_available, failure_available) {
            (true, true) => DiffCoverage::Complete,
            (false, false) => DiffCoverage::Unavailable,
            _ => DiffCoverage::Partial,
        },
        first_difference_boundary: first,
        difference_count,
        success_value: first_values.0,
        failure_value: first_values.1,
        limitation: None,
    }
}

fn compare_objective_domain(
    boundaries: &BTreeSet<u64>,
    success: &BTreeMap<u64, &TraceRecord>,
    failure: &BTreeMap<u64, &TraceRecord>,
    success_evidence: &BTreeMap<u64, Value>,
    failure_evidence: &BTreeMap<u64, Value>,
) -> DomainDiff {
    let values = |record: Option<&TraceRecord>, evidence: Option<&Value>| {
        let goal = available_channel(record, TraceChannel::GoalProgress)
            .and_then(|record| serde_json::to_value(&record.goal_progress).ok());
        (goal.is_some() || evidence.is_some()).then(|| {
            json!({
                "goal_progress": goal,
                "transition_evidence": evidence,
            })
        })
    };
    let mut first = None;
    let mut difference_count = 0;
    let mut first_values = (None, None);
    let mut success_available = false;
    let mut failure_available = false;
    for boundary in boundaries {
        let left = values(
            success.get(boundary).copied(),
            success_evidence.get(boundary),
        );
        let right = values(
            failure.get(boundary).copied(),
            failure_evidence.get(boundary),
        );
        success_available |= left.is_some();
        failure_available |= right.is_some();
        if left != right {
            difference_count += 1;
            if first.is_none() {
                first = Some(*boundary);
                first_values = (left, right);
            }
        }
    }
    DomainDiff {
        coverage: match (success_available, failure_available) {
            (true, true) => DiffCoverage::Complete,
            (false, false) => DiffCoverage::Unavailable,
            _ => DiffCoverage::Partial,
        },
        first_difference_boundary: first,
        difference_count,
        success_value: first_values.0,
        failure_value: first_values.1,
        limitation: None,
    }
}

fn available_channel(record: Option<&TraceRecord>, channel: TraceChannel) -> Option<&TraceRecord> {
    record.filter(|record| {
        matches!(
            record.channel_status.get(&channel),
            Some(TraceChannelStatus::Present | TraceChannelStatus::Absent)
        )
    })
}

fn objective_evidence_by_boundary(
    evidence: Option<&TransitionEvidenceBundle>,
) -> Result<BTreeMap<u64, Value>, TraceDiffError> {
    let mut output = BTreeMap::new();
    if let Some(evidence) = evidence {
        for transition in &evidence.transitions {
            output.insert(
                transition.post_action.boundary_index,
                json!({
                    "predicate": transition.predicate,
                    "reward": transition.reward,
                    "terminal_reason": transition.terminal_reason,
                }),
            );
        }
    }
    Ok(output)
}

#[derive(Debug)]
pub struct TraceDiffError(String);

impl TraceDiffError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for TraceDiffError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for TraceDiffError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tape::TapeBoot;
    use crate::trace::{TraceGoalProgress, TracePhase, TraceRngSnapshot, TraceRngStream};

    fn trace(record: TraceRecord) -> DecodedTrace {
        DecodedTrace {
            version: 5,
            boot: TapeBoot::Process,
            tick_rate_numerator: 30,
            tick_rate_denominator: 1,
            requested_channels: TraceChannel::Rng.bit() | TraceChannel::GoalProgress.bit(),
            capacity_exhausted: false,
            retention: None,
            channel_formats: BTreeMap::new(),
            records: vec![record],
        }
    }

    #[test]
    fn reports_first_rng_and_objective_divergence_and_missing_allocation_fidelity() {
        let record = |calls, reached| {
            let mut record = TraceRecord {
                boundary_index: 4,
                simulation_tick: 3,
                observation_phase: TracePhase::PostSimulation,
                rng: Some(TraceRngSnapshot {
                    version: 1,
                    stream_count: 2,
                    primary: TraceRngStream {
                        id: 0,
                        algorithm_version: 1,
                        state: [1, 2, 3],
                        call_count: calls,
                    },
                    secondary: TraceRngStream {
                        id: 1,
                        algorithm_version: 1,
                        state: [4, 5, 6],
                        call_count: 0,
                    },
                }),
                goal_progress: Some(TraceGoalProgress {
                    configured: true,
                    reached,
                    authored: true,
                    goal_name_hash: Some(1),
                    requested_count: 1,
                    hit_count: u16::from(reached),
                    stable_ticks: 1,
                    consecutive_ticks: u16::from(reached),
                    sequence_steps: 0,
                    sequence_next_step: 0,
                    sequence_within_ticks: 0,
                    sequence_elapsed_ticks: 0,
                    first_hit_tick: reached.then_some(3),
                }),
                ..TraceRecord::default()
            };
            record
                .channel_status
                .insert(TraceChannel::Rng, TraceChannelStatus::Present);
            record
                .channel_status
                .insert(TraceChannel::GoalProgress, TraceChannelStatus::Present);
            record
        };
        let success = trace(record(10, true));
        let failure = trace(record(11, false));
        let diff =
            SiblingTraceDiff::compare(&success, b"success", &failure, b"failure", None, None)
                .unwrap();
        assert_eq!(diff.first_divergence_boundary, Some(4));
        assert_eq!(diff.domains["rng_draw"].first_difference_boundary, Some(4));
        assert_eq!(
            diff.domains["objective_component"].first_difference_boundary,
            Some(4)
        );
        assert_eq!(
            diff.domains["allocation"].coverage,
            DiffCoverage::Unavailable
        );
        assert!(diff.domains["allocation"].limitation.is_some());
    }
}
