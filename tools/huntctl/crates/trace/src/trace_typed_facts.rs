//! Lossless projection of a decoded trace boundary into the shared typed-fact envelope.

use crate::trace::{TraceActorIdentity, TraceChannel, TraceChannelStatus, TracePhase, TraceRecord};
use dusklight_automation_contracts::typed_facts::{
    TYPED_FACT_RESPONSE_MAJOR_VERSION, TYPED_FACT_RESPONSE_MINOR_VERSION,
    TYPED_FACT_RESPONSE_SCHEMA_V1, TypedFactActorIdentity, TypedFactEntry, TypedFactId,
    TypedFactPhase, TypedFactResponse, TypedFactStatus, TypedFactValue, TypedFactValueType,
};

pub fn typed_facts_from_trace_record(record: &TraceRecord) -> TypedFactResponse {
    let stage_status = channel_status(record, TraceChannel::Stage);
    let player_status = channel_status(record, TraceChannel::PlayerMotion);
    let event_status = channel_status(record, TraceChannel::Event);
    let action_status = channel_status(record, TraceChannel::PlayerAction);

    let stage_name = if stage_status == TypedFactStatus::Present {
        let value = TypedFactValue::StageCode(record.stage_name.clone());
        if valid_stage_code(&record.stage_name) {
            entry(
                TypedFactId::StageName,
                TypedFactValueType::StageCode,
                Some(value),
            )
        } else {
            missing(
                TypedFactId::StageName,
                TypedFactValueType::StageCode,
                TypedFactStatus::Invalid,
            )
        }
    } else {
        missing(
            TypedFactId::StageName,
            TypedFactValueType::StageCode,
            stage_status,
        )
    };

    let player_sampled = matches!(
        player_status,
        TypedFactStatus::Present | TypedFactStatus::Absent
    );
    let player_present = player_status == TypedFactStatus::Present && record.player_present();
    let dependent_player_status = if player_sampled {
        if player_present {
            TypedFactStatus::Present
        } else {
            TypedFactStatus::Absent
        }
    } else {
        player_status
    };
    let player_position = if dependent_player_status == TypedFactStatus::Present
        && record.position.iter().all(|value| value.is_finite())
    {
        entry(
            TypedFactId::PlayerPosition,
            TypedFactValueType::Vec3F32,
            Some(TypedFactValue::Vec3F32(
                record.position.map(canonical_float),
            )),
        )
    } else {
        missing(
            TypedFactId::PlayerPosition,
            TypedFactValueType::Vec3F32,
            if dependent_player_status == TypedFactStatus::Present {
                TypedFactStatus::Invalid
            } else {
                dependent_player_status
            },
        )
    };

    let interaction_status = if action_status == TypedFactStatus::Present {
        if let Some(action) = &record.player_action {
            Some(action)
        } else {
            None
        }
    } else {
        None
    };
    let missing_action_status = if action_status == TypedFactStatus::Present {
        TypedFactStatus::Invalid
    } else {
        action_status
    };

    let mut entries = vec![
        stage_name,
        scalar_or_missing(
            TypedFactId::StageRoom,
            TypedFactValueType::I32,
            stage_status,
            TypedFactValue::I32(record.room.into()),
        ),
        scalar_or_missing(
            TypedFactId::StageSpawn,
            TypedFactValueType::I32,
            stage_status,
            TypedFactValue::I32(record.point.into()),
        ),
        if player_sampled {
            entry(
                TypedFactId::PlayerExists,
                TypedFactValueType::Boolean,
                Some(TypedFactValue::Boolean(player_present)),
            )
        } else {
            missing(
                TypedFactId::PlayerExists,
                TypedFactValueType::Boolean,
                player_status,
            )
        },
        scalar_or_missing(
            TypedFactId::PlayerIsLink,
            TypedFactValueType::Boolean,
            dependent_player_status,
            TypedFactValue::Boolean(record.player_is_link()),
        ),
        player_position,
        scalar_or_missing(
            TypedFactId::EventRunning,
            TypedFactValueType::Boolean,
            event_status,
            TypedFactValue::Boolean(record.event_running()),
        ),
        scalar_or_missing(
            TypedFactId::EventId,
            TypedFactValueType::I32,
            event_status,
            TypedFactValue::I32(record.event_id.into()),
        ),
    ];

    if let Some(action) = interaction_status {
        entries.push(entry(
            TypedFactId::PlayerDoStatus,
            TypedFactValueType::U32,
            Some(TypedFactValue::U32(action.do_status.into())),
        ));
        entries.push(actor_entry(
            TypedFactId::TalkPartner,
            action.talk_partner.as_ref(),
        ));
        entries.push(actor_entry(
            TypedFactId::GrabbedActor,
            action.grabbed_actor.as_ref(),
        ));
    } else {
        entries.push(missing(
            TypedFactId::PlayerDoStatus,
            TypedFactValueType::U32,
            missing_action_status,
        ));
        entries.push(missing(
            TypedFactId::TalkPartner,
            TypedFactValueType::ActorIdentity,
            missing_action_status,
        ));
        entries.push(missing(
            TypedFactId::GrabbedActor,
            TypedFactValueType::ActorIdentity,
            missing_action_status,
        ));
    }

    TypedFactResponse {
        schema: TYPED_FACT_RESPONSE_SCHEMA_V1.into(),
        major_version: TYPED_FACT_RESPONSE_MAJOR_VERSION,
        minor_version: TYPED_FACT_RESPONSE_MINOR_VERSION,
        phase: match record.observation_phase {
            TracePhase::PreInput => TypedFactPhase::PreInput,
            TracePhase::PostSimulation => TypedFactPhase::PostSimulation,
        },
        simulation_tick: record.simulation_tick,
        tape_frame: record.tape_frame,
        entries,
    }
}

fn channel_status(record: &TraceRecord, channel: TraceChannel) -> TypedFactStatus {
    match record.channel_status.get(&channel) {
        Some(TraceChannelStatus::Present) => TypedFactStatus::Present,
        Some(TraceChannelStatus::Absent) => TypedFactStatus::Absent,
        Some(TraceChannelStatus::Unavailable | TraceChannelStatus::NotSampled) | None => {
            TypedFactStatus::Unavailable
        }
        Some(TraceChannelStatus::Truncated) => TypedFactStatus::Truncated,
    }
}

fn entry(
    id: TypedFactId,
    value_type: TypedFactValueType,
    value: Option<TypedFactValue>,
) -> TypedFactEntry {
    TypedFactEntry {
        id,
        status: TypedFactStatus::Present,
        value_type,
        value,
    }
}

fn missing(
    id: TypedFactId,
    value_type: TypedFactValueType,
    status: TypedFactStatus,
) -> TypedFactEntry {
    TypedFactEntry {
        id,
        status,
        value_type,
        value: None,
    }
}

fn scalar_or_missing(
    id: TypedFactId,
    value_type: TypedFactValueType,
    status: TypedFactStatus,
    value: TypedFactValue,
) -> TypedFactEntry {
    if status == TypedFactStatus::Present {
        entry(id, value_type, Some(value))
    } else {
        missing(id, value_type, status)
    }
}

fn actor_entry(id: TypedFactId, actor: Option<&TraceActorIdentity>) -> TypedFactEntry {
    match actor {
        Some(actor) => entry(
            id,
            TypedFactValueType::ActorIdentity,
            Some(TypedFactValue::ActorIdentity(TypedFactActorIdentity {
                runtime_generation: actor.session_process_id,
                actor_name: actor.actor_name,
                set_id: actor.set_id,
                home_room: actor.home_room,
                current_room: actor.current_room,
            })),
        ),
        None => missing(
            id,
            TypedFactValueType::ActorIdentity,
            TypedFactStatus::Absent,
        ),
    }
}

fn canonical_float(value: f32) -> f32 {
    if value == 0.0 { 0.0 } else { value }
}

fn valid_stage_code(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 8
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace::{TracePhase, TracePlayerAction};

    fn record() -> TraceRecord {
        let mut record = TraceRecord {
            simulation_tick: 17,
            tape_frame: Some(16),
            observation_phase: TracePhase::PostSimulation,
            stage_name: "F_SP104".into(),
            room: 2,
            point: 3,
            flags: 0b111,
            position: [-0.0, 2.0, 3.0],
            event_id: 9,
            player_action: Some(TracePlayerAction {
                procedure_id: 0,
                mode_flags: 0,
                procedure_context_raw: [0; 6],
                damage_wait_timer: 0,
                sword_at_up_time: 0,
                ice_damage_wait_timer: 0,
                sword_change_wait_timer: 0,
                under_animations: std::array::from_fn(|_| crate::trace::TraceAnimationLane {
                    resource_id: 0,
                    frame: 0.0,
                    rate: 0.0,
                }),
                upper_animations: std::array::from_fn(|_| crate::trace::TraceAnimationLane {
                    resource_id: 0,
                    frame: 0.0,
                    rate: 0.0,
                }),
                do_status: 7,
                talk_partner: Some(TraceActorIdentity {
                    session_process_id: 42,
                    actor_name: 12,
                    set_id: 8,
                    home_room: 2,
                    current_room: 2,
                }),
                grabbed_actor: None,
            }),
            ..TraceRecord::default()
        };
        for channel in [
            TraceChannel::Stage,
            TraceChannel::PlayerMotion,
            TraceChannel::Event,
            TraceChannel::PlayerAction,
        ] {
            record
                .channel_status
                .insert(channel, TraceChannelStatus::Present);
        }
        record
    }

    #[test]
    fn preserves_exact_trace_facts_and_boundary_identity() {
        let response = typed_facts_from_trace_record(&record());
        response.validate().unwrap();
        assert_eq!(response.phase, TypedFactPhase::PostSimulation);
        assert_eq!(response.simulation_tick, 17);
        assert_eq!(response.tape_frame, Some(16));
        assert_eq!(
            response.entries[5].value,
            Some(TypedFactValue::Vec3F32([0.0, 2.0, 3.0]))
        );
        assert!(matches!(
            response.entries[9].value,
            Some(TypedFactValue::ActorIdentity(TypedFactActorIdentity {
                runtime_generation: 42,
                ..
            }))
        ));
        assert_eq!(response.entries[10].status, TypedFactStatus::Absent);
    }

    #[test]
    fn preserves_unavailable_and_truncated_channels_without_values() {
        let mut record = record();
        record
            .channel_status
            .insert(TraceChannel::Event, TraceChannelStatus::Unavailable);
        record
            .channel_status
            .insert(TraceChannel::PlayerAction, TraceChannelStatus::Truncated);
        let response = typed_facts_from_trace_record(&record);
        response.validate().unwrap();
        assert_eq!(response.entries[6].status, TypedFactStatus::Unavailable);
        assert_eq!(response.entries[7].value, None);
        assert_eq!(response.entries[8].status, TypedFactStatus::Truncated);
        assert_eq!(response.entries[9].value, None);
    }

    #[test]
    fn sampled_player_absence_is_a_present_exists_fact() {
        let mut record = record();
        record.flags = 0;
        record
            .channel_status
            .insert(TraceChannel::PlayerMotion, TraceChannelStatus::Absent);
        let response = typed_facts_from_trace_record(&record);
        response.validate().unwrap();
        assert_eq!(response.entries[3].status, TypedFactStatus::Present);
        assert_eq!(
            response.entries[3].value,
            Some(TypedFactValue::Boolean(false))
        );
        assert_eq!(response.entries[4].status, TypedFactStatus::Absent);
        assert_eq!(response.entries[5].status, TypedFactStatus::Absent);
    }
}
