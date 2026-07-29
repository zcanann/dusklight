use super::*;

pub(super) fn evaluate_one(
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

pub(super) fn anomaly_tick(observation: &RunAnomalyObservation) -> u64 {
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
                .then_some(OracleFacts::Action {
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

pub(super) fn vector_length(vector: [f32; 3]) -> f32 {
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
