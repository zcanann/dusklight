use super::*;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RecordedTraceMilestoneHit {
    pub record_index: usize,
    pub boundary_index: u64,
    pub simulation_tick: u64,
    pub tape_frame: Option<u64>,
}

/// Evaluate authored predicates offline over immutable decoded gameplay records.
/// Facts not present in the trace schema (currently actor catalogs and flags)
/// are unavailable and therefore cannot make a comparison true.
pub fn evaluate_recorded_trace(
    program: &MilestoneProgram,
    trace: &crate::trace::DecodedTrace,
) -> Result<BTreeMap<String, Option<RecordedTraceMilestoneHit>>, BinaryError> {
    validate_program(program).map_err(BinaryError)?;
    #[derive(Default)]
    struct State {
        stable: u16,
        sequence_next: usize,
        sequence_elapsed: u16,
        hit: Option<RecordedTraceMilestoneHit>,
    }
    let mut states = (0..program.definitions.len())
        .map(|_| State::default())
        .collect::<Vec<_>>();
    for (record_index, record) in trace.records.iter().enumerate() {
        for (definition, state) in program.definitions.iter().zip(&mut states) {
            if state.hit.is_some()
                || !matches!(
                    (definition.phase, record.observation_phase),
                    (
                        EvaluationPhase::PreInput,
                        crate::trace::TracePhase::PreInput
                    ) | (
                        EvaluationPhase::PostSim,
                        crate::trace::TracePhase::PostSimulation
                    )
                )
            {
                continue;
            }
            let capture = || RecordedTraceMilestoneHit {
                record_index,
                boundary_index: record.boundary_index,
                simulation_tick: record.simulation_tick,
                tape_frame: record.tape_frame,
            };
            if !definition.then.is_empty() {
                let steps = std::iter::once(&definition.when)
                    .chain(&definition.then)
                    .collect::<Vec<_>>();
                if state.sequence_next == 0 {
                    if evaluate_trace_expression(steps[0], record) == Some(true) {
                        state.sequence_next = 1;
                        state.sequence_elapsed = 0;
                    }
                    continue;
                }
                let next_elapsed = state.sequence_elapsed.saturating_add(1);
                if next_elapsed > definition.within_ticks.unwrap() {
                    state.sequence_next =
                        usize::from(evaluate_trace_expression(steps[0], record) == Some(true));
                    state.sequence_elapsed = 0;
                    continue;
                }
                state.sequence_elapsed = next_elapsed;
                if evaluate_trace_expression(steps[state.sequence_next], record) == Some(true) {
                    state.sequence_next += 1;
                } else if evaluate_trace_expression(steps[0], record) == Some(true) {
                    state.sequence_next = 1;
                    state.sequence_elapsed = 0;
                }
                if state.sequence_next == steps.len() {
                    state.hit = Some(capture());
                }
                continue;
            }
            if evaluate_trace_expression(&definition.when, record) == Some(true) {
                state.stable = state.stable.saturating_add(1).min(definition.stable_ticks);
                if state.stable == definition.stable_ticks {
                    state.hit = Some(capture());
                }
            } else {
                state.stable = 0;
            }
        }
    }
    Ok(program
        .definitions
        .iter()
        .zip(states)
        .map(|(definition, state)| (definition.name.clone(), state.hit))
        .collect())
}

fn trace_channel_present(
    record: &crate::trace::TraceRecord,
    channel: crate::trace::TraceChannel,
) -> bool {
    record.channel_status.get(&channel) == Some(&crate::trace::TraceChannelStatus::Present)
}

fn evaluate_trace_expression(
    expression: &Expression,
    record: &crate::trace::TraceRecord,
) -> Option<bool> {
    let facts = typed_facts_from_trace_record(record);
    evaluate_trace_expression_with_facts(expression, record, &facts)
}

fn evaluate_trace_expression_with_facts(
    expression: &Expression,
    record: &crate::trace::TraceRecord,
    facts: &TypedFactResponse,
) -> Option<bool> {
    match expression {
        Expression::Compare {
            field,
            operator,
            value,
        } => trace_field(record, facts, *field)
            .map(|actual| compare_trace_values(&actual, *operator, value)),
        Expression::Query {
            fact,
            operator,
            value,
        } => {
            trace_query(record, fact).map(|actual| compare_trace_values(&actual, *operator, value))
        }
        Expression::Not(inner) => {
            evaluate_trace_expression_with_facts(inner, record, facts).map(|value| !value)
        }
        Expression::And(left, right) => match (
            evaluate_trace_expression_with_facts(left, record, facts),
            evaluate_trace_expression_with_facts(right, record, facts),
        ) {
            (Some(false), _) | (_, Some(false)) => Some(false),
            (Some(true), Some(true)) => Some(true),
            _ => None,
        },
        Expression::Or(left, right) => match (
            evaluate_trace_expression_with_facts(left, record, facts),
            evaluate_trace_expression_with_facts(right, record, facts),
        ) {
            (Some(true), _) | (_, Some(true)) => Some(true),
            (Some(false), Some(false)) => Some(false),
            _ => None,
        },
    }
}

fn trace_field(
    record: &crate::trace::TraceRecord,
    facts: &TypedFactResponse,
    field: Field,
) -> Option<Value> {
    use crate::trace::TraceChannel as Channel;
    let stage = trace_channel_present(record, Channel::Stage);
    let player = trace_channel_present(record, Channel::PlayerMotion);
    let event = trace_channel_present(record, Channel::Event);
    let action = record.player_action.as_ref();
    let rng = record.rng.as_ref();
    let collision = record.player_background_collision.as_ref();
    Some(match field {
        Field::BoundaryKind => Value::U32(u32::from(record.boundary_index != 0)),
        Field::BoundaryIndex => Value::U64(record.boundary_index),
        Field::TapeFrame => Value::U64(record.tape_frame?),
        Field::BoundaryReached => Value::Bool(true),
        Field::StageName => Value::Symbol(typed_stage_code(facts, TypedFactId::StageName)?.into()),
        Field::StageRoom => Value::I32(typed_i32(facts, TypedFactId::StageRoom)?),
        Field::StageLayer if stage => Value::I32(record.layer.into()),
        Field::StageSpawn => Value::I32(typed_i32(facts, TypedFactId::StageSpawn)?),
        Field::NextStageName if stage => Value::Symbol(record.next_stage_name.clone()),
        Field::NextStageRoom if stage => Value::I32(record.next_room.into()),
        Field::NextStageLayer if stage => Value::I32(record.next_layer.into()),
        Field::NextStageSpawn if stage => Value::I32(record.next_point.into()),
        Field::NextStageEnabled if stage => Value::Bool(record.next_stage_enabled),
        Field::PlayerExists => Value::Bool(typed_bool(facts, TypedFactId::PlayerExists)?),
        Field::PlayerIsLink => Value::Bool(typed_bool(facts, TypedFactId::PlayerIsLink)?),
        Field::PlayerProcessId if player => Value::U32(record.player_session_process_id?),
        Field::PlayerActorName if player => Value::I32(record.player_actor_name.into()),
        Field::PlayerPositionX => Value::F32(typed_vec3(facts, TypedFactId::PlayerPosition)?[0]),
        Field::PlayerPositionY => Value::F32(typed_vec3(facts, TypedFactId::PlayerPosition)?[1]),
        Field::PlayerPositionZ => Value::F32(typed_vec3(facts, TypedFactId::PlayerPosition)?[2]),
        Field::PlayerVelocityX if player => Value::F32(record.velocity[0]),
        Field::PlayerVelocityY if player => Value::F32(record.velocity[1]),
        Field::PlayerVelocityZ if player => Value::F32(record.velocity[2]),
        Field::PlayerSpeed if player => Value::F32(record.forward_speed),
        Field::PlayerProcedure if player => Value::ProcedureNumber(record.player_proc_id?.into()),
        Field::PlayerCurrentAngleX if player => Value::I32(record.current_angle[0].into()),
        Field::PlayerCurrentAngleY if player => Value::I32(record.current_angle[1].into()),
        Field::PlayerCurrentAngleZ if player => Value::I32(record.current_angle[2].into()),
        Field::PlayerShapeAngleX if player => Value::I32(record.shape_angle[0].into()),
        Field::PlayerShapeAngleY if player => Value::I32(record.shape_angle[1].into()),
        Field::PlayerShapeAngleZ if player => Value::I32(record.shape_angle[2].into()),
        Field::PlayerModeFlags => Value::U32(action?.mode_flags),
        Field::PlayerDamageWaitTimer => Value::I32(action?.damage_wait_timer.into()),
        Field::PlayerIceDamageWaitTimer => Value::I32(action?.ice_damage_wait_timer.into()),
        Field::PlayerSwordChangeWaitTimer => Value::U32(action?.sword_change_wait_timer.into()),
        Field::EventRunning => Value::Bool(typed_bool(facts, TypedFactId::EventRunning)?),
        Field::EventId => Value::I32(typed_i32(facts, TypedFactId::EventId)?),
        Field::EventMode if event => Value::U32(record.event_mode.into()),
        Field::EventStatus if event => Value::U32(record.event_status.into()),
        Field::EventMapToolId if event => Value::U32(record.event_map_tool_id.into()),
        Field::EventNameHashPresent if event => Value::Bool(record.event_name_hash_present),
        Field::EventNameHash if event && record.event_name_hash_present => {
            Value::U32(record.event_name_hash)
        }
        Field::RngPrimaryState0 => Value::I32(rng?.primary.state[0]),
        Field::RngPrimaryState1 => Value::I32(rng?.primary.state[1]),
        Field::RngPrimaryState2 => Value::I32(rng?.primary.state[2]),
        Field::RngPrimaryCalls => Value::U64(rng?.primary.call_count),
        Field::RngSecondaryState0 => Value::I32(rng?.secondary.state[0]),
        Field::RngSecondaryState1 => Value::I32(rng?.secondary.state[1]),
        Field::RngSecondaryState2 => Value::I32(rng?.secondary.state[2]),
        Field::RngSecondaryCalls => Value::U64(rng?.secondary.call_count),
        Field::CollisionGroundContact => Value::Bool(collision?.flags & (1 << 1) != 0),
        Field::CollisionWallContact => Value::Bool(collision?.flags & (1 << 6) != 0),
        Field::CollisionRoofContact => Value::Bool(collision?.flags & (1 << 8) != 0),
        Field::CollisionWaterContact => Value::Bool(collision?.flags & (1 << 11) != 0),
        Field::CollisionWaterIn => Value::Bool(collision?.flags & (1 << 12) != 0),
        Field::CollisionGroundHeight => Value::F32(collision?.ground_height),
        Field::CollisionRoofHeight => Value::F32(collision?.roof_height),
        Field::CollisionGroundClearance if player => {
            Value::F32(record.position[1] - collision?.ground_height)
        }
        Field::PlayerDoStatus => Value::U32(typed_u32(facts, TypedFactId::PlayerDoStatus)?),
        Field::TalkPartnerExists => {
            Value::Bool(typed_actor_exists(facts, TypedFactId::TalkPartner)?)
        }
        Field::TalkPartnerActorName => Value::I32(
            typed_actor(facts, TypedFactId::TalkPartner)?
                .actor_name
                .into(),
        ),
        Field::TalkPartnerSetId => {
            Value::U32(typed_actor(facts, TypedFactId::TalkPartner)?.set_id.into())
        }
        Field::TalkPartnerHomeRoom => Value::I32(
            typed_actor(facts, TypedFactId::TalkPartner)?
                .home_room
                .into(),
        ),
        Field::TalkPartnerCurrentRoom => Value::I32(
            typed_actor(facts, TypedFactId::TalkPartner)?
                .current_room
                .into(),
        ),
        Field::GrabbedActorExists => {
            Value::Bool(typed_actor_exists(facts, TypedFactId::GrabbedActor)?)
        }
        Field::GrabbedActorActorName => Value::I32(
            typed_actor(facts, TypedFactId::GrabbedActor)?
                .actor_name
                .into(),
        ),
        Field::GrabbedActorSetId => {
            Value::U32(typed_actor(facts, TypedFactId::GrabbedActor)?.set_id.into())
        }
        Field::GrabbedActorHomeRoom => Value::I32(
            typed_actor(facts, TypedFactId::GrabbedActor)?
                .home_room
                .into(),
        ),
        Field::GrabbedActorCurrentRoom => Value::I32(
            typed_actor(facts, TypedFactId::GrabbedActor)?
                .current_room
                .into(),
        ),
        Field::TalkPartnerHomePositionX => {
            Value::F32(typed_actor(facts, TypedFactId::TalkPartner)?.home_position?[0])
        }
        Field::TalkPartnerHomePositionY => {
            Value::F32(typed_actor(facts, TypedFactId::TalkPartner)?.home_position?[1])
        }
        Field::TalkPartnerHomePositionZ => {
            Value::F32(typed_actor(facts, TypedFactId::TalkPartner)?.home_position?[2])
        }
        Field::GrabbedActorHomePositionX => {
            Value::F32(typed_actor(facts, TypedFactId::GrabbedActor)?.home_position?[0])
        }
        Field::GrabbedActorHomePositionY => {
            Value::F32(typed_actor(facts, TypedFactId::GrabbedActor)?.home_position?[1])
        }
        Field::GrabbedActorHomePositionZ => {
            Value::F32(typed_actor(facts, TypedFactId::GrabbedActor)?.home_position?[2])
        }
        _ => return None,
    })
}

fn typed_value(facts: &TypedFactResponse, id: TypedFactId) -> Option<&TypedFactValue> {
    let entry = facts
        .entries
        .binary_search_by_key(&id, |entry| entry.id)
        .ok()
        .map(|index| &facts.entries[index])?;
    (entry.status == TypedFactStatus::Present)
        .then_some(entry.value.as_ref())
        .flatten()
}

fn typed_bool(facts: &TypedFactResponse, id: TypedFactId) -> Option<bool> {
    match typed_value(facts, id)? {
        TypedFactValue::Boolean(value) => Some(*value),
        _ => None,
    }
}

fn typed_i32(facts: &TypedFactResponse, id: TypedFactId) -> Option<i32> {
    match typed_value(facts, id)? {
        TypedFactValue::I32(value) => Some(*value),
        _ => None,
    }
}

fn typed_u32(facts: &TypedFactResponse, id: TypedFactId) -> Option<u32> {
    match typed_value(facts, id)? {
        TypedFactValue::U32(value) => Some(*value),
        _ => None,
    }
}

fn typed_vec3(facts: &TypedFactResponse, id: TypedFactId) -> Option<[f32; 3]> {
    match typed_value(facts, id)? {
        TypedFactValue::Vec3F32(value) => Some(*value),
        _ => None,
    }
}

fn typed_stage_code(facts: &TypedFactResponse, id: TypedFactId) -> Option<&str> {
    match typed_value(facts, id)? {
        TypedFactValue::StageCode(value) => Some(value),
        _ => None,
    }
}

fn typed_actor(facts: &TypedFactResponse, id: TypedFactId) -> Option<&TypedFactActorIdentity> {
    match typed_value(facts, id)? {
        TypedFactValue::ActorIdentity(value) => Some(value),
        _ => None,
    }
}

fn typed_actor_exists(facts: &TypedFactResponse, id: TypedFactId) -> Option<bool> {
    let entry = facts
        .entries
        .binary_search_by_key(&id, |entry| entry.id)
        .ok()
        .map(|index| &facts.entries[index])?;
    match entry.status {
        TypedFactStatus::Present => Some(true),
        TypedFactStatus::Absent => Some(false),
        _ => None,
    }
}

fn trace_query(record: &crate::trace::TraceRecord, fact: &QueryFact) -> Option<Value> {
    match fact {
        QueryFact::PlayerInAabb { minimum, maximum } if record.player_present() => {
            Some(Value::Bool((0..3).all(|axis| {
                record.position[axis] >= minimum[axis] && record.position[axis] <= maximum[axis]
            })))
        }
        QueryFact::PlayerPlaneSignedDistance { point, normal } if record.player_present() => {
            let length =
                (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
            Some(Value::F32(
                ((record.position[0] - point[0]) * normal[0]
                    + (record.position[1] - point[1]) * normal[1]
                    + (record.position[2] - point[2]) * normal[2])
                    / length,
            ))
        }
        _ => None,
    }
}

fn compare_trace_values(actual: &Value, operator: Comparison, expected: &Value) -> bool {
    macro_rules! ordered {
        ($left:expr, $right:expr) => {
            match operator {
                Comparison::Equal => $left == $right,
                Comparison::NotEqual => $left != $right,
                Comparison::Less => $left < $right,
                Comparison::LessEqual => $left <= $right,
                Comparison::Greater => $left > $right,
                Comparison::GreaterEqual => $left >= $right,
                Comparison::HasAll | Comparison::HasAny => false,
            }
        };
    }
    match (actual, expected) {
        (Value::Bool(left), Value::Bool(right)) => ordered!(*left, *right),
        (Value::U32(left), Value::U32(right)) => match operator {
            Comparison::HasAll => left & right == *right,
            Comparison::HasAny => left & right != 0,
            _ => ordered!(*left, *right),
        },
        (Value::U64(left), Value::U64(right)) => match operator {
            Comparison::HasAll => left & right == *right,
            Comparison::HasAny => left & right != 0,
            _ => ordered!(*left, *right),
        },
        (Value::I32(left), Value::I32(right)) => ordered!(*left, *right),
        (Value::F32(left), Value::F32(right)) => match operator {
            Comparison::Equal => left.to_bits() == right.to_bits(),
            Comparison::NotEqual => left.to_bits() != right.to_bits(),
            _ => ordered!(*left, *right),
        },
        (Value::Symbol(left), Value::Symbol(right)) => ordered!(left, right),
        (Value::U32(left), Value::Symbol(right)) if *left <= 1 => {
            let expected = match right.as_str() {
                "boot" => 0,
                "tick" => 1,
                _ => return false,
            };
            ordered!(*left, expected)
        }
        (Value::ProcedureNumber(left), Value::ProcedureNumber(right)) => ordered!(*left, *right),
        _ => false,
    }
}
