
use super::*;
use crate::identity::RUNTIME_CONFIGURATION_SCHEMA;
use crate::native_observation::{
    NativeActorIdentity, NativeActorObservation, NativeAttentionCandidateObservation,
    NativeAttentionCandidatesObservation, NativeEventActorReferenceObservation,
    NativeEventQueueObservation, NativeMessageFlowObservation, NativePendingEventOrderObservation,
    NativePhysicalSlotObservation, NativePlayerActionObservation, NativePlayerControlObservation,
    NativePlayerRelationshipsObservation, NativeReturnPlaceWriterObservation,
    NativeRuntimeFileObservation,
};

fn context(sequence: u64) -> NativeSnapshotContext {
    NativeSnapshotContext {
        snapshot_id: format!("native.snapshot.{sequence}"),
        sequence,
        runtime_configuration: RuntimeConfiguration {
            schema: RUNTIME_CONFIGURATION_SCHEMA.into(),
            content_sha256: Digest([1; 32]),
            language: "en".into(),
            settings: BTreeMap::new(),
        },
        runtime_file_id: "runtime.fixture".into(),
        session_id: "session.fixture".into(),
        evidence_id: "native.fixture.v14".into(),
        evidence_sha256: Digest([2; 32]),
    }
}

fn observation() -> NativeLearningObservation {
    NativeLearningObservation {
        stage: "F_SP103".into(),
        room: 0,
        layer: 0,
        point: 1,
        player_present: true,
        player_is_link: true,
        player_position: [1.0, 2.0, 3.0],
        player_attention_position: Some([4.0, 5.0, 6.0]),
        player_current_angle: [0, 0x1000, 0],
        player_form_present: true,
        player_action: Some(NativePlayerActionObservation {
            procedure_id: 0x1234,
        }),
        runtime_file_status: NativeChannelStatus::Present,
        runtime_file: Some(NativeRuntimeFileObservation {
            no_file_raw: 1,
            data_num_raw: 2,
            backing_attachment_status: NativeChannelStatus::Present,
            attached_physical_slot: Some(2),
            physical_slots: [
                NativePhysicalSlotObservation {
                    number: 1,
                    ..Default::default()
                },
                NativePhysicalSlotObservation {
                    number: 2,
                    attached_to_runtime: true,
                    ..Default::default()
                },
                NativePhysicalSlotObservation {
                    number: 3,
                    ..Default::default()
                },
            ],
        }),
        persistent_event_bytes: Some(vec![0; 256]),
        player_light_drop_bytes: Some(vec![0; 5]),
        event_flags: Some(vec![0; 822]),
        temporary_flags: Some(vec![0; 185]),
        temporary_event_bytes: Some(vec![0; 256]),
        loaded_stage_memory_bytes: Some(vec![0; 0x20]),
        event_handoff_status: NativeChannelStatus::Present,
        event_handoff: Some(NativeEventHandoffObservation {
            get_item_no: 0x43,
            message_flow_status: NativeChannelStatus::Present,
            message_flow: Some(NativeMessageFlowObservation {
                flow_id: 7,
                node_index: 2,
                cut_name_hash: 0,
            }),
            message_cut_status: NativeChannelStatus::Unavailable,
            ..Default::default()
        }),
        message_session_status: NativeChannelStatus::Present,
        message_session: Some(NativeMessageSessionObservation {
            procedure: 6,
            message_id: 0x123456,
            message_index: 17,
            node_index: 9,
            flow_id: 0x777,
            selection_count: 3,
            selection_cursor: 1,
            selection_push: 2,
            output_type: 4,
            talk_now: true,
            talk_message: true,
            send: true,
            talk_actor: NativeActorIdentity {
                present: true,
                runtime_generation: 7,
                actor_name: 0x123,
            },
            ..Default::default()
        }),
        event_queue_status: NativeChannelStatus::Present,
        event_queue: Some(NativeEventQueueObservation {
            pending_orders: vec![NativePendingEventOrderObservation {
                event_type: 0,
                event_id: 12,
                priority: 2,
                map_tool_id: 3,
                request_actor: NativeEventActorReferenceObservation {
                    status: NativeChannelStatus::Present,
                    actor: Some(NativeActorIdentity {
                        present: true,
                        runtime_generation: 42,
                        actor_name: 0x123,
                    }),
                },
                target_actor: NativeEventActorReferenceObservation {
                    status: NativeChannelStatus::Absent,
                    actor: None,
                },
                ..Default::default()
            }],
            active_request_actor: NativeEventActorReferenceObservation {
                status: NativeChannelStatus::Present,
                actor: Some(NativeActorIdentity {
                    present: true,
                    runtime_generation: 42,
                    actor_name: 0x123,
                }),
            },
            skip_actor: NativeEventActorReferenceObservation {
                status: NativeChannelStatus::Absent,
                actor: None,
            },
            ..Default::default()
        }),
        attention_candidates_status: NativeChannelStatus::Present,
        attention_candidates: Some(NativeAttentionCandidatesObservation {
            player_attention_flags: 0x1234,
            attention_status: 2,
            attention_block_timer: 3,
            action_candidates: vec![NativeAttentionCandidateObservation {
                actor: NativeEventActorReferenceObservation {
                    status: NativeChannelStatus::Present,
                    actor: Some(NativeActorIdentity {
                        present: true,
                        runtime_generation: 42,
                        actor_name: 0x123,
                    }),
                },
                weight: 0.5,
                distance: 90.0,
                angle: 0x200,
                attention_type: 6,
            }],
            ..Default::default()
        }),
        actors: vec![NativeActorObservation {
            runtime_generation: 42,
            return_place_writer: Some(NativeReturnPlaceWriterObservation {
                save_room: 3,
                required_switch_set: 8,
                ..Default::default()
            }),
        }],
        player_relationships_status: NativeChannelStatus::Present,
        player_relationships: Some(NativePlayerRelationshipsObservation {
            ride_actor: Some(NativeActorIdentity {
                present: true,
                runtime_generation: 5,
                actor_name: 0x123,
            }),
        }),
        ..Default::default()
    }
}

#[test]
fn projects_native_backing_components_writers_and_explicit_unknowns() {
    let snapshot = snapshot_native_observation(&observation(), context(1)).unwrap();
    snapshot.validate().unwrap();
    assert_eq!(snapshot.environment.player.position, [1.0, 2.0, 3.0]);
    assert_eq!(
        snapshot.environment.player.attention_position,
        Some([4.0, 5.0, 6.0])
    );
    assert_eq!(
        snapshot.environment.active_runtime_file.backing,
        BackingAttachment::CardBacked {
            slot: PhysicalSlotId(2)
        }
    );
    assert!(snapshot.environment.physical_slots.is_empty());
    assert_eq!(snapshot.environment.physical_slot_observations.len(), 3);
    assert!(
        snapshot
            .environment
            .physical_slot_observations
            .iter()
            .all(|slot| slot.content_status == CaptureStatus::NotSampled)
    );
    let handoff = snapshot
        .environment
        .components
        .iter()
        .find(|component| component.id == "event-handoff")
        .unwrap();
    let ComponentPayload::Structured { fields } = &handoff.payload else {
        panic!("event handoff must be structured");
    };
    assert_eq!(
        fields["message_flow_status"],
        StateValue::Text("present".into())
    );
    assert_eq!(
        fields["message_cut_status"],
        StateValue::Text("unavailable".into())
    );
    assert!(!fields.contains_key("get_item_no"));
    let recent_item = snapshot
        .environment
        .components
        .iter()
        .find(|component| component.id == "event-recent-item")
        .unwrap();
    assert_eq!(recent_item.component_kind, ComponentKind::Session);
    assert_eq!(recent_item.lifetime, SemanticLifetime::Session);
    let ComponentPayload::Structured { fields } = &recent_item.payload else {
        panic!("recent item must be structured");
    };
    assert_eq!(fields["get_item_no"], StateValue::Unsigned(0x43));
    let writer = snapshot
        .environment
        .components
        .iter()
        .find(|component| component.id.ends_with(".return-place-writer"))
        .unwrap();
    assert_eq!(writer.component_kind, ComponentKind::ActorInstance);
    let ComponentPayload::Structured { fields } = &writer.payload else {
        panic!("return-place writer must be structured");
    };
    assert_eq!(fields["save_room"], StateValue::Signed(3));
    assert_eq!(fields["required_switch_set"], StateValue::Unsigned(8));
    assert_eq!(fields["no_telop_clear"], StateValue::Boolean(false));
    assert_eq!(fields["eligible"], StateValue::Boolean(false));
    assert_eq!(snapshot.environment.live_world_objects.len(), 1);
    assert_eq!(
        snapshot.environment.live_world_objects[0].actor_type,
        "kytag14.return-place-writer"
    );
}

#[test]
fn dungeon_resources_remain_in_the_bound_stage_bank_not_runtime_inventory() {
    let mut observation = observation();
    observation.stage = "D_MN05".into();
    observation.player_resources_status = NativeChannelStatus::Present;
    observation.player_resources = Some(NativePlayerResourcesObservation {
        maximum_oil: Some(21_600),
        oil: Some(1_234),
        small_keys: 3,
        dungeon_map: true,
        dungeon_compass: true,
        dungeon_boss_key: true,
        dungeon_warp: true,
        ..Default::default()
    });
    let mut stage_memory = vec![0_u8; 0x20];
    stage_memory[0x1c] = 3;
    stage_memory[0x1d] = 0b0100_0111;
    observation.loaded_stage_memory_bytes = Some(stage_memory);

    let snapshot = snapshot_native_observation(&observation, context(1)).unwrap();
    let inventory = snapshot
        .environment
        .components
        .iter()
        .find(|component| component.id == "inventory-and-resources")
        .unwrap();
    let ComponentPayload::Structured { fields } = &inventory.payload else {
        panic!("inventory must be structured");
    };
    for local_field in [
        "small_keys",
        "dungeon_map",
        "dungeon_compass",
        "dungeon_boss_key",
        "dungeon_warp",
    ] {
        assert!(!fields.contains_key(local_field));
    }
    assert_eq!(fields["maximum_oil"], StateValue::Unsigned(21_600));
    assert_eq!(fields["oil"], StateValue::Unsigned(1_234));

    let dungeon = snapshot
        .environment
        .components
        .iter()
        .find(|component| component.id == "flags.loaded-stage-memory")
        .unwrap();
    assert_eq!(dungeon.component_kind, ComponentKind::DungeonMemory);
    assert_eq!(
        dungeon.binding,
        ComponentBinding::Stage {
            stage: "D_MN05".into()
        }
    );
    assert_eq!(
        dungeon.serialization_owner,
        SerializationOwner::StageBank {
            runtime_file_id: "runtime.fixture".into(),
            stage: "D_MN05".into()
        }
    );
    let ComponentPayload::Raw { bytes, .. } = &dungeon.payload else {
        panic!("stage memory must retain raw bytes");
    };
    assert_eq!(bytes.len(), 0x20);
    assert_eq!(bytes[0x1c], 3);
    assert_eq!(bytes[0x1d], 0b0100_0111);
}

#[test]
fn refuses_to_invent_a_runtime_or_player_for_missing_channels() {
    let mut observation = observation();
    observation.runtime_file_status = NativeChannelStatus::NotSampled;
    observation.runtime_file = None;
    let error = snapshot_native_observation(&observation, context(1)).unwrap_err();
    assert_eq!(error.field(), "native_observation.runtime_file");
}

#[test]
fn projects_global_message_session_as_generic_flow_state() {
    let snapshot = snapshot_native_observation(&observation(), context(1)).unwrap();
    let message = snapshot
        .environment
        .components
        .iter()
        .find(|component| component.id == "message-session")
        .unwrap();
    assert_eq!(message.component_kind, ComponentKind::MessageFlow);
    let ComponentPayload::Structured { fields } = &message.payload else {
        panic!("message session must be structured");
    };
    assert_eq!(fields["capture_status"], StateValue::Text("present".into()));
    assert_eq!(fields["message_id"], StateValue::Unsigned(0x123456));
    assert_eq!(fields["flow_id"], StateValue::Signed(0x777));
    assert_eq!(fields["node_index"], StateValue::Unsigned(9));
    assert_eq!(fields["talk_now"], StateValue::Boolean(true));
    assert_eq!(
        fields["talk_actor_runtime_generation"],
        StateValue::Unsigned(7)
    );
}

#[test]
fn projects_complete_event_handoff_without_collapsing_storage_lifetimes() {
    let mut observation = observation();
    observation.temporary_event_bytes.as_mut().unwrap()[5] = 0xa5;
    let handoff = observation.event_handoff.as_mut().unwrap();
    handoff.pre_item_no = 0x90;
    handoff.get_item_no = 0x4a;
    handoff.event_name_status = NativeChannelStatus::Present;
    handoff.event_name = Some("DEFAULT_GETITEM".into());
    handoff.message_cut_status = NativeChannelStatus::Present;
    handoff.message_flow.as_mut().unwrap().cut_name_hash = 0x1234_5678;
    handoff.pending_cleanup_status = NativeChannelStatus::Present;
    handoff.pending_cleanup_flags = Some(0xa5a5_5a5a);
    handoff.player_control_status = NativeChannelStatus::Present;
    handoff.player_control = Some(NativePlayerControlObservation {
        mode_flags: 0x1020_3040,
        do_status: 7,
    });
    handoff.item_partner = NativeActorIdentity {
        present: true,
        runtime_generation: 11,
        actor_name: 0x123,
    };

    let snapshot = snapshot_native_observation(&observation, context(1)).unwrap();
    let structured = |id: &str| {
        let component = snapshot
            .environment
            .components
            .iter()
            .find(|component| component.id == id)
            .unwrap();
        let ComponentPayload::Structured { fields } = &component.payload else {
            panic!("{id} must be structured")
        };
        fields
    };
    let event = structured("event-handoff");
    assert_eq!(event["pre_item_no"], StateValue::Unsigned(0x90));
    assert_eq!(event["flow_id"], StateValue::Unsigned(7));
    assert_eq!(event["node_index"], StateValue::Unsigned(2));
    assert_eq!(event["cut_name_hash"], StateValue::Unsigned(0x1234_5678));
    assert_eq!(
        event["event_name"],
        StateValue::Text("DEFAULT_GETITEM".into())
    );
    assert_eq!(
        event["pending_cleanup_flags"],
        StateValue::Unsigned(0xa5a5_5a5a)
    );
    assert_eq!(
        event["player_mode_flags"],
        StateValue::Unsigned(0x1020_3040)
    );
    assert_eq!(event["player_do_status"], StateValue::Unsigned(7));
    assert_eq!(
        event["item_partner_runtime_generation"],
        StateValue::Unsigned(11)
    );
    assert!(!event.contains_key("get_item_no"));

    let recent = structured("event-recent-item");
    assert_eq!(recent["get_item_no"], StateValue::Unsigned(0x4a));
    let temporary = snapshot
        .environment
        .components
        .iter()
        .find(|component| component.id == "flags.temporary-event-registers")
        .unwrap();
    let ComponentPayload::Raw { bytes, .. } = &temporary.payload else {
        panic!("temporary message progress must retain raw bytes")
    };
    assert_eq!(bytes[5], 0xa5);
}

#[test]
fn projects_event_requests_and_participants_without_native_pointers() {
    let snapshot = snapshot_native_observation(&observation(), context(1)).unwrap();
    let queue = snapshot
        .environment
        .components
        .iter()
        .find(|component| component.id == "event-queue")
        .unwrap();
    assert_eq!(queue.component_kind, ComponentKind::PendingOperation);
    let ComponentPayload::Structured { fields } = &queue.payload else {
        panic!("event queue must be structured");
    };
    assert_eq!(fields["capture_status"], StateValue::Text("present".into()));
    assert_eq!(fields["pending_count"], StateValue::Unsigned(1));
    assert_eq!(fields["pending.0.event_type"], StateValue::Unsigned(0));
    assert_eq!(fields["pending.0.priority"], StateValue::Unsigned(2));
    assert_eq!(
        fields["pending.0.request_actor.runtime_generation"],
        StateValue::Unsigned(42)
    );
    assert_eq!(
        fields["pending.0.target_actor.status"],
        StateValue::Text("absent".into())
    );
}

#[test]
fn projects_attention_candidates_without_selecting_one() {
    let snapshot = snapshot_native_observation(&observation(), context(1)).unwrap();
    let attention = snapshot
        .environment
        .components
        .iter()
        .find(|component| component.id == "attention-candidates")
        .unwrap();
    assert_eq!(attention.component_kind, ComponentKind::PendingOperation);
    let ComponentPayload::Structured { fields } = &attention.payload else {
        panic!("attention candidates must be structured");
    };
    assert_eq!(fields["capture_status"], StateValue::Text("present".into()));
    assert_eq!(fields["action.count"], StateValue::Unsigned(1));
    assert_eq!(fields["action.0.attention_type"], StateValue::Unsigned(6));
    assert_eq!(
        fields["action.0.actor.runtime_generation"],
        StateValue::Unsigned(42)
    );
    assert_eq!(
        fields["action.0.distance_f32_bits"],
        StateValue::Unsigned(90.0_f32.to_bits().into())
    );
}

#[test]
fn separates_label_observations_from_writable_event_register_backings() {
    let snapshot = snapshot_native_observation(&observation(), context(1)).unwrap();
    let persistent_registers = snapshot
        .environment
        .components
        .iter()
        .find(|component| component.id == "flags.persistent-event-registers")
        .unwrap();
    assert_eq!(
        persistent_registers.component_kind,
        ComponentKind::Custom {
            id: "persistent-event-registers".into()
        }
    );
    assert_eq!(
        persistent_registers.binding,
        ComponentBinding::RuntimeFile {
            runtime_file_id: "runtime.fixture".into()
        }
    );
    assert!(matches!(
        persistent_registers.payload,
        ComponentPayload::Raw { ref bytes, .. } if bytes.len() == 256
    ));
    let light_drop = snapshot
        .environment
        .components
        .iter()
        .find(|component| component.id == "save.player-light-drop")
        .unwrap();
    assert_eq!(
        light_drop.component_kind,
        ComponentKind::Custom {
            id: "player-light-drop".into()
        }
    );
    assert_eq!(light_drop.binding, persistent_registers.binding);
    assert!(matches!(
        light_drop.payload,
        ComponentPayload::Raw { ref bytes, .. } if bytes.len() == 5
    ));
    let event_labels = snapshot
        .environment
        .components
        .iter()
        .find(|component| component.id == "flags.event")
        .unwrap();
    assert_eq!(
        event_labels.component_kind,
        ComponentKind::Custom {
            id: "observed-event-flag-labels".into()
        }
    );
    let temporary_labels = snapshot
        .environment
        .components
        .iter()
        .find(|component| component.id == "flags.temporary")
        .unwrap();
    assert_eq!(
        temporary_labels.component_kind,
        ComponentKind::Custom {
            id: "observed-temporary-flag-labels".into()
        }
    );
    let temporary_registers = snapshot
        .environment
        .components
        .iter()
        .find(|component| component.id == "flags.temporary-event-registers")
        .unwrap();
    assert_eq!(
        temporary_registers.component_kind,
        ComponentKind::TemporaryFlags
    );
    assert_eq!(
        snapshot
            .environment
            .components
            .iter()
            .filter(|component| {
                component.component_kind == ComponentKind::TemporaryFlags
                    && matches!(component.payload, ComponentPayload::Raw { .. })
            })
            .count(),
        1
    );

    let dungeon_session_labels = snapshot
        .environment
        .components
        .iter()
        .find(|component| component.id == "flags.dungeon-session-labels")
        .unwrap();
    assert_eq!(
        dungeon_session_labels.component_kind,
        ComponentKind::Custom {
            id: "observed-dungeon-session-switch-labels".into()
        }
    );
    let loaded_stage_memory = snapshot
        .environment
        .components
        .iter()
        .find(|component| component.id == "flags.loaded-stage-memory")
        .unwrap();
    assert_eq!(
        loaded_stage_memory.component_kind,
        ComponentKind::DungeonMemory
    );
    let room_switch_labels = snapshot
        .environment
        .components
        .iter()
        .find(|component| component.id == "flags.room-switch-labels")
        .unwrap();
    assert_eq!(
        room_switch_labels.component_kind,
        ComponentKind::Custom {
            id: "observed-room-switch-labels".into()
        }
    );
    assert_eq!(
        snapshot
            .environment
            .components
            .iter()
            .filter(|component| {
                component.component_kind == ComponentKind::DungeonMemory
                    && matches!(component.payload, ComponentPayload::Raw { .. })
            })
            .count(),
        1
    );
}

#[test]
fn rejects_partial_persistent_event_register_captures() {
    let mut partial_event = observation();
    partial_event.persistent_event_bytes = Some(vec![0; 255]);
    assert_eq!(
        snapshot_native_observation(&partial_event, context(1))
            .unwrap_err()
            .field(),
        "native_observation.persistent_event_bytes"
    );

    let mut partial_light_drop = observation();
    partial_light_drop.player_light_drop_bytes = Some(vec![0; 4]);
    assert_eq!(
        snapshot_native_observation(&partial_light_drop, context(1))
            .unwrap_err()
            .field(),
        "native_observation.player_light_drop_bytes"
    );
}

#[test]
fn chains_label_boundaries_and_retain_exact_raw_byte_diffs() {
    let first = observation();
    let mut second = first.clone();
    second.temporary_event_bytes.as_mut().unwrap()[19] ^= 0x04;
    second
        .event_handoff
        .as_mut()
        .unwrap()
        .message_flow
        .as_mut()
        .unwrap()
        .node_index += 1;

    let mut evidence = NativeStateEvidence::begin(&first, context(1)).unwrap();
    evidence
        .append(&second, context(2), BoundaryKind::DialogueInterruption)
        .unwrap();
    assert_eq!(evidence.snapshots.len(), 2);
    assert_eq!(evidence.diffs.len(), 1);
    assert_eq!(evidence.chain.entries.len(), 2);
    assert_eq!(
        evidence.chain.entries[1].incoming_boundary,
        Some(BoundaryKind::DialogueInterruption)
    );
    let temporary_delta = evidence.diffs[0]
        .component_deltas
        .iter()
        .find(|delta| delta.component_id == "flags.temporary-event-registers")
        .unwrap();
    assert_eq!(temporary_delta.raw_byte_deltas.len(), 1);
    assert_eq!(temporary_delta.raw_byte_deltas[0].offset, 19);
    assert!(
        evidence.diffs[0]
            .component_deltas
            .iter()
            .any(|delta| delta.component_id == "event-handoff")
    );
    evidence.chain.validate().unwrap();
}
