
use super::*;
use crate::identity::{RUNTIME_CONFIGURATION_SCHEMA, RuntimeConfiguration};
use crate::snapshot::STATE_SNAPSHOT_SCHEMA;
use crate::state::{
    BOUNDARY_POLICY_SCHEMA, BackingAttachment, BoundaryKind, ComponentBindingProjection,
    ComponentBindingReference, ComponentBoundaryRule, EXECUTION_ENVIRONMENT_SCHEMA,
    ExecutionEnvironment, PersistedObjectControl, PhysicalSlotId, PlayerForm, PlayerMount,
    PlayerState, RuntimeFile, RuntimeFileLifecycle, RuntimeFileOrigin, SceneLocation,
    SemanticLifetime, StaticWorldObject,
};
use crate::transition::ComponentFieldTarget;

fn provenance() -> Vec<ComponentProvenance> {
    vec![ComponentProvenance {
        source_kind: ProvenanceSourceKind::Initialized,
        source_id: "fixture.initial".into(),
        source_sha256: Some(Digest([7; 32])),
        transition_id: None,
    }]
}

fn structured_component(
    id: &str,
    kind: ComponentKind,
    binding: ComponentBinding,
) -> StateComponent {
    StateComponent {
        id: id.into(),
        component_kind: kind,
        payload: ComponentPayload::Structured {
            fields: BTreeMap::new(),
        },
        binding,
        lifetime: SemanticLifetime::RuntimeFile,
        serialization_owner: SerializationOwner::RuntimeFile {
            runtime_file_id: "file-0".into(),
        },
        provenance: provenance(),
    }
}

fn raw_component() -> StateComponent {
    StateComponent {
        id: "raw.flags".into(),
        component_kind: ComponentKind::PersistentSave,
        payload: ComponentPayload::Raw {
            bytes: vec![0],
            known_mask: vec![0],
        },
        binding: ComponentBinding::Global,
        lifetime: SemanticLifetime::RuntimeFile,
        serialization_owner: SerializationOwner::RuntimeFile {
            runtime_file_id: "file-0".into(),
        },
        provenance: provenance(),
    }
}

fn snapshot() -> StateSnapshot {
    let mut flow = structured_component(
        "flow.main",
        ComponentKind::MessageFlow,
        ComponentBinding::Session {
            session_id: "session-1".into(),
        },
    );
    let ComponentPayload::Structured { fields } = &mut flow.payload else {
        unreachable!()
    };
    fields.insert("node_id".into(), StateValue::Text("start".into()));

    StateSnapshot {
        schema: STATE_SNAPSHOT_SCHEMA.into(),
        id: "snapshot.before".into(),
        sequence: 4,
        environment: ExecutionEnvironment {
            schema: EXECUTION_ENVIRONMENT_SCHEMA.into(),
            runtime_configuration: RuntimeConfiguration {
                schema: RUNTIME_CONFIGURATION_SCHEMA.into(),
                content_sha256: Digest([1; 32]),
                language: "en".into(),
                settings: BTreeMap::new(),
            },
            active_runtime_file: RuntimeFile {
                id: "file-0".into(),
                origin: RuntimeFileOrigin::TitleFile0,
                backing: BackingAttachment::MemoryOnly,
                allowed_serialization_targets: vec![PhysicalSlotId(1)],
                lifecycle: RuntimeFileLifecycle::Active,
            },
            inactive_runtime_files: Vec::new(),
            physical_slots: Vec::new(),
            physical_slot_observations: Vec::new(),
            execution_context: ExecutionContext::World,
            location: SceneLocation {
                stage: "F_SP103".into(),
                room: 0,
                layer: 0,
                spawn: 0,
            },
            player: PlayerState {
                form: PlayerForm::Human,
                mount: None,
                position: [0.0; 3],
                attention_position: None,
                rotation: [0; 3],
                has_control: Some(true),
                action: "idle".into(),
            },
            components: vec![
                flow,
                structured_component(
                    "pending.item",
                    ComponentKind::PendingOperation,
                    ComponentBinding::Session {
                        session_id: "session-1".into(),
                    },
                ),
                raw_component(),
                structured_component(
                    "save.main",
                    ComponentKind::PersistentSave,
                    ComponentBinding::RuntimeFile {
                        runtime_file_id: "file-0".into(),
                    },
                ),
            ],
            static_world_objects: Vec::new(),
            spatial_volumes: Vec::new(),
            spatial_connections: Vec::new(),
            spatial_planes: Vec::new(),
            persisted_object_controls: Vec::new(),
            live_world_objects: Vec::new(),
        },
        semantic_observations: Vec::new(),
    }
}

fn id_selector(component_id: &str) -> ComponentSelector {
    ComponentSelector::Id {
        component_id: component_id.into(),
    }
}

fn field<'a>(state: &'a PlannerExecutionState, component_id: &str, name: &str) -> &'a StateValue {
    let component = state
        .snapshot
        .environment
        .components
        .iter()
        .find(|component| component.id == component_id)
        .unwrap();
    let ComponentPayload::Structured { fields } = &component.payload else {
        panic!("expected structured component")
    };
    fields.get(name).unwrap()
}

#[test]
fn applies_writes_gates_and_locations_as_one_new_snapshot() {
    let mut state = PlannerExecutionState::new(snapshot()).unwrap();
    let result = state
        .apply_operations(
            "transition.enter-forest",
            "snapshot.after-enter-forest",
            &[
                StateOperation::Write {
                    target: ComponentFieldTarget {
                        component_id: "save.main".into(),
                        field: "small_keys".into(),
                    },
                    value: StateValue::Unsigned(1),
                },
                StateOperation::SetGate {
                    gate_id: "gate.no-teleport".into(),
                },
                StateOperation::SetLocation {
                    location: SceneLocation {
                        stage: "D_MN05".into(),
                        room: 1,
                        layer: 0,
                        spawn: 2,
                    },
                },
            ],
        )
        .unwrap();
    assert_ne!(result.source_snapshot_sha256, result.result_snapshot_sha256);
    assert_eq!(state.snapshot.sequence, 5);
    assert_eq!(state.snapshot.id, "snapshot.after-enter-forest");
    assert_eq!(state.snapshot.environment.location.stage, "D_MN05");
    assert_eq!(
        field(&state, "save.main", "small_keys"),
        &StateValue::Unsigned(1)
    );
    assert_eq!(state.gate_states.get("gate.no-teleport"), Some(&true));
    assert_eq!(state.execution_history.len(), 3);
    let mut without_history = state.clone();
    without_history.execution_history.clear();
    assert_ne!(state.digest().unwrap(), without_history.digest().unwrap());
    assert_eq!(
        state.semantic_digest().unwrap(),
        without_history.semantic_digest().unwrap()
    );
    assert_eq!(
        state
            .last_field_writer("save.main", "small_keys")
            .unwrap()
            .application_id,
        "transition.enter-forest"
    );
    assert_eq!(state.gate_history("gate.no-teleport").len(), 1);
    assert_eq!(
        state
            .snapshot
            .environment
            .components
            .iter()
            .find(|component| component.id == "save.main")
            .unwrap()
            .provenance
            .last()
            .unwrap()
            .transition_id
            .as_deref(),
        Some("transition.enter-forest")
    );
}

#[test]
fn multi_field_write_updates_one_record_atomically_and_tracks_each_field() {
    let mut state = PlannerExecutionState::new(snapshot()).unwrap();
    state
        .apply_operations(
            "writer.savmem-tower",
            "snapshot.after-savmem-tower",
            &[StateOperation::WriteFields {
                component_id: "save.main".into(),
                fields: BTreeMap::from([
                    ("return_stage".into(), StateValue::Text("R_SP107".into())),
                    ("return_room".into(), StateValue::Signed(3)),
                    ("return_spawn".into(), StateValue::Unsigned(1)),
                ]),
            }],
        )
        .unwrap();

    assert_eq!(
        field(&state, "save.main", "return_stage"),
        &StateValue::Text("R_SP107".into())
    );
    assert_eq!(
        field(&state, "save.main", "return_room"),
        &StateValue::Signed(3)
    );
    assert_eq!(
        field(&state, "save.main", "return_spawn"),
        &StateValue::Unsigned(1)
    );
    for name in ["return_stage", "return_room", "return_spawn"] {
        assert_eq!(
            state
                .last_field_writer("save.main", name)
                .unwrap()
                .application_id,
            "writer.savmem-tower"
        );
    }

    state
        .apply_operations(
            "transition.savewarp",
            "snapshot.after-savewarp",
            &[StateOperation::SetLocationFromFields {
                component_id: "save.main".into(),
                stage_field: "return_stage".into(),
                room_field: "return_room".into(),
                spawn_field: "return_spawn".into(),
                layer: -1,
            }],
        )
        .unwrap();
    assert_eq!(state.snapshot.environment.location.stage, "R_SP107");
    assert_eq!(state.snapshot.environment.location.room, 3);
    assert_eq!(state.snapshot.environment.location.spawn, 1);
    assert_eq!(state.snapshot.environment.location.layer, -1);
}

#[test]
fn structured_invalidation_removes_known_value_with_distinct_provenance() {
    let mut state = PlannerExecutionState::new(snapshot()).unwrap();
    state
        .apply_operations(
            "cutscene.unknown-suffix",
            "snapshot.after-unknown-suffix",
            &[StateOperation::InvalidateField {
                target: ComponentFieldTarget {
                    component_id: "save.main".into(),
                    field: "small_keys".into(),
                },
            }],
        )
        .unwrap();
    let component = state
        .snapshot
        .environment
        .components
        .iter()
        .find(|component| component.id == "save.main")
        .unwrap();
    let ComponentPayload::Structured { fields } = &component.payload else {
        unreachable!()
    };
    assert!(!fields.contains_key("small_keys"));
    assert!(matches!(
        &state.execution_history.last().unwrap().event,
        ExecutionHistoryKind::Operation {
            operation: StateOperation::InvalidateField { .. },
            ..
        }
    ));
}

#[test]
fn player_state_operations_are_ordered_and_round_trip_in_history() {
    let mut state = PlannerExecutionState::new(snapshot()).unwrap();
    state
        .apply_operations(
            "cutscene.partial-player-state",
            "snapshot.partial-player-state",
            &[
                StateOperation::SetPlayerForm {
                    form: PlayerForm::Wolf,
                },
                StateOperation::SetPlayerMount {
                    mount: Some(PlayerMount::Epona),
                },
                StateOperation::SetPlayerControl { has_control: None },
                StateOperation::SetPlayerAction {
                    action: "cutscene-warp".into(),
                },
            ],
        )
        .unwrap();

    assert_eq!(state.snapshot.environment.player.form, PlayerForm::Wolf);
    assert_eq!(
        state.snapshot.environment.player.mount,
        Some(PlayerMount::Epona)
    );
    assert_eq!(state.snapshot.environment.player.has_control, None);
    assert_eq!(state.snapshot.environment.player.action, "cutscene-warp");
    assert_eq!(state.execution_history.len(), 4);
    assert!(state.execution_history.iter().all(|event| {
        matches!(
            &event.event,
            ExecutionHistoryKind::Operation {
                affected_component_ids,
                ..
            } if affected_component_ids.is_empty()
        )
    }));

    let document = state.to_document().unwrap();
    assert_eq!(document.schema, PLANNER_EXECUTION_STATE_SCHEMA);
    let decoded =
        PlannerExecutionStateDocument::decode_canonical(&document.canonical_bytes().unwrap())
            .unwrap();
    assert_eq!(decoded.into_state().unwrap(), state);
}

#[test]
fn held_writer_value_and_gate_history_remain_queryable_in_order() {
    let mut state = PlannerExecutionState::new(snapshot()).unwrap();
    let return_place = ComponentFieldTarget {
        component_id: "save.main".into(),
        field: "player_return_place".into(),
    };
    state
        .apply_operations(
            "writer.return-place.ordon",
            "snapshot.return-place.ordon",
            &[StateOperation::Write {
                target: return_place.clone(),
                value: StateValue::Text("F_SP103:0:0:0".into()),
            }],
        )
        .unwrap();
    state
        .apply_operations(
            "gate.fanadi-lock.set",
            "snapshot.fanadi-lock.set",
            &[StateOperation::SetGate {
                gate_id: "gate.no-telop".into(),
            }],
        )
        .unwrap();

    assert_eq!(
        field(&state, "save.main", "player_return_place"),
        &StateValue::Text("F_SP103:0:0:0".into())
    );
    assert_eq!(
        state
            .last_field_writer("save.main", "player_return_place")
            .unwrap()
            .application_id,
        "writer.return-place.ordon"
    );
    let gate_history = state.gate_history("gate.no-telop");
    assert_eq!(gate_history.len(), 1);
    assert_eq!(gate_history[0].application_id, "gate.fanadi-lock.set");

    state
        .apply_operations(
            "gate.fanadi-lock.release-and-write",
            "snapshot.fanadi-lock.released",
            &[
                StateOperation::ClearGate {
                    gate_id: "gate.no-telop".into(),
                },
                StateOperation::Write {
                    target: return_place,
                    value: StateValue::Text("R_SP109:0:0:0".into()),
                },
            ],
        )
        .unwrap();
    assert_eq!(state.gate_history("gate.no-telop").len(), 2);
    let last_writer = state
        .last_field_writer("save.main", "player_return_place")
        .unwrap();
    assert_eq!(
        last_writer.application_id,
        "gate.fanadi-lock.release-and-write"
    );
    assert_eq!(last_writer.operation_index, 1);
}

#[test]
fn recent_item_survives_file_load_and_drives_generic_inventory_grant() {
    // dItemNo_FISHING_ROD_1_e and dItemNo_RAFRELS_MEMO_e.
    const ROD_ITEM_ID: u64 = 0x4a;
    const MEMO_ITEM_ID: u64 = 0x90;

    let mut source = snapshot();
    let mut recent_item = structured_component(
        "event.recent-item",
        ComponentKind::Session,
        ComponentBinding::Session {
            session_id: "session-1".into(),
        },
    );
    recent_item.lifetime = SemanticLifetime::Session;
    recent_item.serialization_owner = SerializationOwner::None;
    let ComponentPayload::Structured { fields } = &mut recent_item.payload else {
        unreachable!()
    };
    fields.insert("get_item_no".into(), StateValue::Unsigned(0));

    let mut handoff = structured_component(
        "event.item-handoff",
        ComponentKind::PendingOperation,
        ComponentBinding::Session {
            session_id: "session-1".into(),
        },
    );
    handoff.lifetime = SemanticLifetime::Action;
    handoff.serialization_owner = SerializationOwner::None;
    let ComponentPayload::Structured { fields } = &mut handoff.payload else {
        unreachable!()
    };
    fields.insert("pre_item_no".into(), StateValue::Unsigned(3));

    let mut inventory_a = structured_component(
        "inventory.active",
        ComponentKind::Inventory,
        ComponentBinding::RuntimeFile {
            runtime_file_id: "file-0".into(),
        },
    );
    let ComponentPayload::Structured { fields } = &mut inventory_a.payload else {
        unreachable!()
    };
    fields.insert("owned_item_ids".into(), StateValue::Bytes(vec![0; 32]));
    source
        .environment
        .components
        .extend([recent_item, handoff, inventory_a]);
    source
        .environment
        .components
        .sort_by(|left, right| left.id.cmp(&right.id));

    let mut state = PlannerExecutionState::new(source).unwrap();
    state
        .apply_operations(
            "writer.file-a-rod-presentation",
            "snapshot.file-a-rod-prepared",
            &[StateOperation::Write {
                target: ComponentFieldTarget {
                    component_id: "event.recent-item".into(),
                    field: "get_item_no".into(),
                },
                value: StateValue::Unsigned(ROD_ITEM_ID),
            }],
        )
        .unwrap();

    let mut inventory_b = structured_component(
        "inventory.active",
        ComponentKind::Inventory,
        ComponentBinding::RuntimeFile {
            runtime_file_id: "file-b".into(),
        },
    );
    inventory_b.serialization_owner = SerializationOwner::RuntimeFile {
        runtime_file_id: "file-b".into(),
    };
    let ComponentPayload::Structured { fields } = &mut inventory_b.payload else {
        unreachable!()
    };
    fields.insert("owned_item_ids".into(), StateValue::Bytes(vec![0; 32]));
    let policy = BoundaryPolicy {
        schema: BOUNDARY_POLICY_SCHEMA.into(),
        id: "boundary.load-file-b".into(),
        boundary: BoundaryKind::LoadPhysicalSlot,
        default_disposition: BoundaryDisposition::Clear,
        component_rules: vec![
            ComponentBoundaryRule {
                selector: id_selector("event.recent-item"),
                disposition: BoundaryDisposition::Preserve,
            },
            ComponentBoundaryRule {
                selector: id_selector("inventory.active"),
                disposition: BoundaryDisposition::Reinitialize {
                    initializer_id: "inventory.active".into(),
                },
            },
        ],
    };
    state
        .apply_boundary(
            "boundary.load-file-b",
            "snapshot.file-b-loaded",
            &policy,
            &BTreeMap::from([("inventory.active".into(), inventory_b)]),
        )
        .unwrap();
    assert_eq!(
        field(&state, "event.recent-item", "get_item_no"),
        &StateValue::Unsigned(ROD_ITEM_ID)
    );
    assert!(
        state
            .snapshot
            .environment
            .components
            .iter()
            .all(|component| component.id != "event.item-handoff")
    );

    let loaded = state.clone();
    let file_b = RuntimeFile {
        id: "file-b".into(),
        origin: RuntimeFileOrigin::LoadedSlot {
            slot: PhysicalSlotId(1),
        },
        backing: BackingAttachment::CardBacked {
            slot: PhysicalSlotId(1),
        },
        allowed_serialization_targets: vec![PhysicalSlotId(1)],
        lifecycle: RuntimeFileLifecycle::Active,
    };
    state
        .apply_operations(
            "auru.broken-generic-get-item",
            "snapshot.file-b-rod-granted",
            &[
                StateOperation::SetActiveRuntimeFile {
                    runtime_file: file_b.clone(),
                },
                StateOperation::SetBitFromValue {
                    source: ComponentFieldTarget {
                        component_id: "event.recent-item".into(),
                        field: "get_item_no".into(),
                    },
                    target: ComponentFieldTarget {
                        component_id: "inventory.active".into(),
                        field: "owned_item_ids".into(),
                    },
                },
            ],
        )
        .unwrap();
    let StateValue::Bytes(items) = field(&state, "inventory.active", "owned_item_ids") else {
        unreachable!()
    };
    assert_ne!(
        items[ROD_ITEM_ID as usize / 8] & (1 << (ROD_ITEM_ID % 8)),
        0
    );

    let mut normal_path = loaded;
    normal_path
        .apply_operations(
            "auru.normal-memo-get-item",
            "snapshot.file-b-memo-granted",
            &[
                StateOperation::SetActiveRuntimeFile {
                    runtime_file: file_b,
                },
                StateOperation::Write {
                    target: ComponentFieldTarget {
                        component_id: "event.recent-item".into(),
                        field: "get_item_no".into(),
                    },
                    value: StateValue::Unsigned(MEMO_ITEM_ID),
                },
                StateOperation::SetBitFromValue {
                    source: ComponentFieldTarget {
                        component_id: "event.recent-item".into(),
                        field: "get_item_no".into(),
                    },
                    target: ComponentFieldTarget {
                        component_id: "inventory.active".into(),
                        field: "owned_item_ids".into(),
                    },
                },
            ],
        )
        .unwrap();
    let StateValue::Bytes(items) = field(&normal_path, "inventory.active", "owned_item_ids") else {
        unreachable!()
    };
    assert_ne!(
        items[MEMO_ITEM_ID as usize / 8] & (1 << (MEMO_ITEM_ID % 8)),
        0
    );
    assert_eq!(
        items[ROD_ITEM_ID as usize / 8] & (1 << (ROD_ITEM_ID % 8)),
        0
    );
}

#[test]
fn recent_item_boundary_matrix_is_process_owned_and_last_writer_wins() {
    const ROD_ITEM_ID: u64 = 0x4a;
    const MEMO_ITEM_ID: u64 = 0x90;

    let recent_item_component = |value: u64| {
        let mut component = structured_component(
            "event.recent-item",
            ComponentKind::Session,
            ComponentBinding::Session {
                session_id: "session-1".into(),
            },
        );
        component.lifetime = SemanticLifetime::Session;
        component.serialization_owner = SerializationOwner::None;
        let ComponentPayload::Structured { fields } = &mut component.payload else {
            unreachable!()
        };
        fields.insert("get_item_no".into(), StateValue::Unsigned(value));
        component
    };
    let state_with_recent_item = || {
        let mut source = snapshot();
        source
            .environment
            .components
            .push(recent_item_component(ROD_ITEM_ID));
        source
            .environment
            .components
            .sort_by(|left, right| left.id.cmp(&right.id));
        PlannerExecutionState::new(source).unwrap()
    };

    let in_process_boundaries = vec![
        ("room-transition", BoundaryKind::RoomTransition),
        ("stage-transition", BoundaryKind::StageTransition),
        ("void-reload", BoundaryKind::VoidReload),
        ("savewarp", BoundaryKind::SaveWarp),
        ("title-return", BoundaryKind::TitleReturn),
        ("load-physical-slot", BoundaryKind::LoadPhysicalSlot),
        ("save-runtime-to-slot", BoundaryKind::SaveRuntimeToSlot),
        ("wrong-state-respawn", BoundaryKind::WrongStateRespawn),
        ("dialogue-interruption", BoundaryKind::DialogueInterruption),
    ];
    for (label, boundary) in in_process_boundaries {
        let mut state = state_with_recent_item();
        let policy = BoundaryPolicy {
            schema: BOUNDARY_POLICY_SCHEMA.into(),
            id: format!("boundary.auru-{label}"),
            boundary,
            default_disposition: BoundaryDisposition::Clear,
            component_rules: vec![ComponentBoundaryRule {
                selector: id_selector("event.recent-item"),
                disposition: BoundaryDisposition::Preserve,
            }],
        };
        state
            .apply_boundary(
                &policy.id,
                &format!("snapshot.after-{label}"),
                &policy,
                &BTreeMap::new(),
            )
            .unwrap();
        assert_eq!(
            field(&state, "event.recent-item", "get_item_no"),
            &StateValue::Unsigned(ROD_ITEM_ID),
            "{label} must not silently clear process-owned mGtItm"
        );
    }

    let mut event_cleanup = state_with_recent_item();
    let mut shown_item = structured_component(
        "event.shown-item",
        ComponentKind::PendingOperation,
        ComponentBinding::Session {
            session_id: "session-1".into(),
        },
    );
    shown_item.lifetime = SemanticLifetime::Action;
    shown_item.serialization_owner = SerializationOwner::None;
    let ComponentPayload::Structured { fields } = &mut shown_item.payload else {
        unreachable!()
    };
    fields.insert("pre_item_no".into(), StateValue::Unsigned(0x91));
    event_cleanup
        .snapshot
        .environment
        .components
        .push(shown_item);
    event_cleanup
        .snapshot
        .environment
        .components
        .sort_by(|left, right| left.id.cmp(&right.id));
    event_cleanup.snapshot.validate().unwrap();
    event_cleanup
        .apply_operations(
            "writer.show-item-x",
            "snapshot.after-show-item-x",
            &[StateOperation::Write {
                target: ComponentFieldTarget {
                    component_id: "event.shown-item".into(),
                    field: "pre_item_no".into(),
                },
                value: StateValue::Unsigned(0x4b),
            }],
        )
        .unwrap();
    assert_eq!(
        field(&event_cleanup, "event.shown-item", "pre_item_no"),
        &StateValue::Unsigned(0x4b)
    );
    assert_eq!(
        field(&event_cleanup, "event.recent-item", "get_item_no"),
        &StateValue::Unsigned(ROD_ITEM_ID),
        "show-item acceptance writes mPreItemNo, not mGtItm"
    );
    let cleanup_policy = BoundaryPolicy {
        schema: BOUNDARY_POLICY_SCHEMA.into(),
        id: "boundary.event-control-remove".into(),
        boundary: BoundaryKind::Custom {
            id: "event-control-remove".into(),
        },
        default_disposition: BoundaryDisposition::Clear,
        component_rules: vec![ComponentBoundaryRule {
            selector: id_selector("event.recent-item"),
            disposition: BoundaryDisposition::Preserve,
        }],
    };
    event_cleanup
        .apply_boundary(
            &cleanup_policy.id,
            "snapshot.after-event-control-remove",
            &cleanup_policy,
            &BTreeMap::new(),
        )
        .unwrap();
    assert!(
        event_cleanup
            .snapshot
            .environment
            .components
            .iter()
            .all(|component| component.id != "event.shown-item")
    );
    assert_eq!(
        field(&event_cleanup, "event.recent-item", "get_item_no"),
        &StateValue::Unsigned(ROD_ITEM_ID)
    );

    event_cleanup
        .apply_operations(
            "writer.auru-normal-memo-presentation",
            "snapshot.after-memo-presentation",
            &[StateOperation::Write {
                target: ComponentFieldTarget {
                    component_id: "event.recent-item".into(),
                    field: "get_item_no".into(),
                },
                value: StateValue::Unsigned(MEMO_ITEM_ID),
            }],
        )
        .unwrap();
    assert_eq!(
        field(&event_cleanup, "event.recent-item", "get_item_no"),
        &StateValue::Unsigned(MEMO_ITEM_ID)
    );
    assert_eq!(
        event_cleanup
            .last_field_writer("event.recent-item", "get_item_no")
            .unwrap()
            .application_id,
        "writer.auru-normal-memo-presentation"
    );

    let process_restart = BoundaryPolicy {
        schema: BOUNDARY_POLICY_SCHEMA.into(),
        id: "boundary.process-restart".into(),
        boundary: BoundaryKind::ProcessRestart,
        default_disposition: BoundaryDisposition::Clear,
        component_rules: vec![ComponentBoundaryRule {
            selector: id_selector("event.recent-item"),
            disposition: BoundaryDisposition::Reinitialize {
                initializer_id: "event.recent-item".into(),
            },
        }],
    };
    event_cleanup
        .apply_boundary(
            &process_restart.id,
            "snapshot.after-process-restart",
            &process_restart,
            &BTreeMap::from([("event.recent-item".into(), recent_item_component(0))]),
        )
        .unwrap();
    assert_eq!(
        field(&event_cleanup, "event.recent-item", "get_item_no"),
        &StateValue::Unsigned(0)
    );
}

#[test]
fn failed_batches_are_atomic() {
    let mut state = PlannerExecutionState::new(snapshot()).unwrap();
    let before = state.clone();
    let error = state
        .apply_operations(
            "transition.bad",
            "snapshot.never-committed",
            &[
                StateOperation::Write {
                    target: ComponentFieldTarget {
                        component_id: "save.main".into(),
                        field: "would_have_changed".into(),
                    },
                    value: StateValue::Boolean(true),
                },
                StateOperation::ClearComponent {
                    selector: id_selector("missing.component"),
                },
            ],
        )
        .unwrap_err();
    assert_eq!(error.field(), "operation.clear_component");
    assert_eq!(state, before);
}

#[test]
fn unsigned_minimum_clamp_raises_only_values_below_the_floor() {
    let target = ComponentFieldTarget {
        component_id: "save.main".into(),
        field: "life".into(),
    };
    let mut low = PlannerExecutionState::new(snapshot()).unwrap();
    low.apply_operations(
        "fixture.low-life",
        "snapshot.low-life",
        &[StateOperation::Write {
            target: target.clone(),
            value: StateValue::Unsigned(4),
        }],
    )
    .unwrap();
    low.apply_operations(
        "normalizer.life-floor",
        "snapshot.life-clamped",
        &[StateOperation::ClampUnsignedMinimum {
            target: target.clone(),
            minimum: 12,
        }],
    )
    .unwrap();
    assert_eq!(field(&low, "save.main", "life"), &StateValue::Unsigned(12));
    assert_eq!(
        low.last_field_writer("save.main", "life")
            .unwrap()
            .application_id,
        "normalizer.life-floor"
    );

    let mut high = PlannerExecutionState::new(snapshot()).unwrap();
    high.apply_operations(
        "fixture.high-life",
        "snapshot.high-life",
        &[StateOperation::Write {
            target: target.clone(),
            value: StateValue::Unsigned(20),
        }],
    )
    .unwrap();
    high.apply_operations(
        "normalizer.life-floor",
        "snapshot.life-preserved",
        &[StateOperation::ClampUnsignedMinimum {
            target,
            minimum: 12,
        }],
    )
    .unwrap();
    assert_eq!(field(&high, "save.main", "life"), &StateValue::Unsigned(20));
    assert_eq!(
        high.last_field_writer("save.main", "life")
            .unwrap()
            .application_id,
        "fixture.high-life"
    );
}

#[test]
fn unsigned_debit_subtracts_or_saturates_at_zero() {
    let target = ComponentFieldTarget {
        component_id: "save.main".into(),
        field: "rupees".into(),
    };
    let mut state = PlannerExecutionState::new(snapshot()).unwrap();
    state
        .apply_operations(
            "fixture.rupees",
            "snapshot.rupees-500",
            &[StateOperation::Write {
                target: target.clone(),
                value: StateValue::Unsigned(500),
            }],
        )
        .unwrap();
    state
        .apply_operations(
            "message.debit-300",
            "snapshot.rupees-200",
            &[StateOperation::DebitUnsigned {
                target: target.clone(),
                amount: 300,
            }],
        )
        .unwrap();
    assert_eq!(
        field(&state, "save.main", "rupees"),
        &StateValue::Unsigned(200)
    );

    state
        .apply_operations(
            "message.debit-300-again",
            "snapshot.rupees-zero",
            &[StateOperation::DebitUnsigned {
                target,
                amount: 300,
            }],
        )
        .unwrap();
    assert_eq!(
        field(&state, "save.main", "rupees"),
        &StateValue::Unsigned(0)
    );
    assert_eq!(
        state
            .last_field_writer("save.main", "rupees")
            .unwrap()
            .application_id,
        "message.debit-300-again"
    );
}

#[test]
fn item_slot_normalization_migrates_items_and_rebuilds_the_lineup() {
    let operation = StateOperation::NormalizeItemSlotsAndLineup {
        component_id: "save.main".into(),
        inventory_field: "inventory".into(),
        lineup_field: "item_lineup".into(),
        primary_slot: 9,
        secondary_slot: 10,
        single_item: 0x44,
        combined_item: 0x47,
        empty_item: 0xff,
        lineup_order: vec![10, 4, 9],
    };
    let mut state = PlannerExecutionState::new(snapshot()).unwrap();
    let mut inventory = vec![0xff; 24];
    inventory[4] = 0x22;
    inventory[9] = 0x47;
    state
        .apply_operations(
            "fixture.legacy-item-layout",
            "snapshot.legacy-item-layout",
            &[StateOperation::WriteFields {
                component_id: "save.main".into(),
                fields: BTreeMap::from([
                    ("inventory".into(), StateValue::Bytes(inventory)),
                    ("item_lineup".into(), StateValue::Bytes(vec![23; 24])),
                ]),
            }],
        )
        .unwrap();
    state
        .apply_operations(
            "normalizer.item-layout",
            "snapshot.normalized-item-layout",
            std::slice::from_ref(&operation),
        )
        .unwrap();

    let StateValue::Bytes(inventory) = field(&state, "save.main", "inventory") else {
        unreachable!()
    };
    assert_eq!(inventory[9], 0xff);
    assert_eq!(inventory[10], 0x47);
    let StateValue::Bytes(lineup) = field(&state, "save.main", "item_lineup") else {
        unreachable!()
    };
    assert_eq!(&lineup[..3], &[10, 4, 0xff]);
    assert!(lineup[3..].iter().all(|item| *item == 0xff));
    for field_name in ["inventory", "item_lineup"] {
        assert_eq!(
            state
                .last_field_writer("save.main", field_name)
                .unwrap()
                .application_id,
            "normalizer.item-layout"
        );
    }

    let mut redundant_single = vec![0xff; 24];
    redundant_single[9] = 0x44;
    redundant_single[10] = 0x47;
    state
        .apply_operations(
            "fixture.redundant-single",
            "snapshot.redundant-single",
            &[StateOperation::WriteFields {
                component_id: "save.main".into(),
                fields: BTreeMap::from([
                    ("inventory".into(), StateValue::Bytes(redundant_single)),
                    ("item_lineup".into(), StateValue::Bytes(vec![0xff; 24])),
                ]),
            }],
        )
        .unwrap();
    state
        .apply_operations(
            "normalizer.redundant-single",
            "snapshot.redundant-single-removed",
            &[operation],
        )
        .unwrap();
    let StateValue::Bytes(inventory) = field(&state, "save.main", "inventory") else {
        unreachable!()
    };
    assert_eq!(inventory[9], 0xff);
    assert_eq!(inventory[10], 0x47);
}

#[test]
fn payload_replacement_retains_component_identity_and_is_a_whole_component_writer() {
    let mut state = PlannerExecutionState::new(snapshot()).unwrap();
    let before = state
        .snapshot
        .environment
        .components
        .iter()
        .find(|component| component.id == "save.main")
        .unwrap()
        .clone();
    let replacement = ComponentPayload::Structured {
        fields: BTreeMap::from([
            ("life".into(), StateValue::Unsigned(12)),
            ("rupees".into(), StateValue::Unsigned(0)),
        ]),
    };

    state
        .apply_operations(
            "initializer.opening-save",
            "snapshot.opening-save-initialized",
            &[StateOperation::ReplacePayload {
                component_id: "save.main".into(),
                payload: replacement.clone(),
            }],
        )
        .unwrap();

    let after = state
        .snapshot
        .environment
        .components
        .iter()
        .find(|component| component.id == "save.main")
        .unwrap();
    assert_eq!(after.payload, replacement);
    assert_eq!(after.id, before.id);
    assert_eq!(after.component_kind, before.component_kind);
    assert_eq!(after.binding, before.binding);
    assert_eq!(after.lifetime, before.lifetime);
    assert_eq!(after.serialization_owner, before.serialization_owner);
    assert_eq!(
        &after.provenance[..before.provenance.len()],
        &before.provenance
    );
    assert_eq!(
        state
            .last_field_writer("save.main", "life")
            .unwrap()
            .application_id,
        "initializer.opening-save"
    );
}

#[test]
fn payload_invalidation_can_include_runtime_stores_but_never_physical_images() {
    let mut state = PlannerExecutionState::new(snapshot()).unwrap();
    state
        .apply_operations(
            "boundary.save-file-0",
            "snapshot.file-0-saved",
            &[StateOperation::SaveRuntimeToSlot {
                source_runtime_file_id: "file-0".into(),
                destination_slot: PhysicalSlotId(1),
                destination_persistent_file_id: "persistent-slot-1".into(),
                runtime_component_ids: vec!["raw.flags".into(), "save.main".into()],
                stage_bank_stages: Vec::new(),
            }],
        )
        .unwrap();
    let sealed_image = state.persistent_file_images["persistent-slot-1"].clone();
    let sealed_digest = sealed_image.digest().unwrap();

    let owner = SerializationOwner::StageBank {
        runtime_file_id: "file-0".into(),
        stage: "F_SP103".into(),
    };
    let mut stored = raw_component();
    stored.binding = ComponentBinding::Stage {
        stage: "F_SP103".into(),
    };
    stored.lifetime = SemanticLifetime::StageLoad;
    stored.serialization_owner = owner.clone();
    state
        .serialized_components
        .insert(owner.clone(), vec![stored]);
    let inactive_owner = SerializationOwner::StageBank {
        runtime_file_id: "inactive-file".into(),
        stage: "F_SP103".into(),
    };
    let mut inactive_stored = raw_component();
    inactive_stored.binding = ComponentBinding::Stage {
        stage: "F_SP103".into(),
    };
    inactive_stored.lifetime = SemanticLifetime::StageLoad;
    inactive_stored.serialization_owner = inactive_owner.clone();
    let inactive_payload = inactive_stored.payload.clone();
    state
        .serialized_components
        .insert(inactive_owner.clone(), vec![inactive_stored]);
    state
        .snapshot
        .environment
        .inactive_runtime_files
        .push(RuntimeFile {
            id: "inactive-file".into(),
            origin: RuntimeFileOrigin::NewFile,
            backing: BackingAttachment::MemoryOnly,
            allowed_serialization_targets: Vec::new(),
            lifecycle: RuntimeFileLifecycle::Suspended,
        });
    state.validate().unwrap();

    state
        .apply_operations(
            "initializer.invalidate-runtime-payloads",
            "snapshot.runtime-payloads-invalidated",
            &[StateOperation::InvalidatePayloads {
                selector: id_selector("raw.flags"),
                include_active_runtime_serialized_stores: true,
            }],
        )
        .unwrap();

    let live = state
        .snapshot
        .environment
        .components
        .iter()
        .find(|component| component.id == "raw.flags")
        .unwrap();
    assert_eq!(
        live.payload,
        ComponentPayload::Unknown {
            expected_bytes: Some(1)
        }
    );
    assert_eq!(
        state.serialized_components[&owner][0].payload,
        ComponentPayload::Unknown {
            expected_bytes: Some(1)
        }
    );
    assert_eq!(
        state.serialized_components[&inactive_owner][0].payload,
        inactive_payload
    );
    assert_eq!(
        state.persistent_file_images["persistent-slot-1"],
        sealed_image
    );
    assert_eq!(
        state.persistent_file_images["persistent-slot-1"]
            .digest()
            .unwrap(),
        sealed_digest
    );
    assert!(matches!(
        state
            .snapshot
            .environment
            .components
            .iter()
            .find(|component| component.id == "save.main")
            .unwrap()
            .payload,
        ComponentPayload::Structured { .. }
    ));
    assert_eq!(
        state
            .last_field_writer("raw.flags", "any-field")
            .unwrap()
            .application_id,
        "initializer.invalidate-runtime-payloads"
    );

    let before_failure = state.clone();
    let error = state
        .apply_operations(
            "initializer.missing-payload",
            "snapshot.not-produced",
            &[StateOperation::InvalidatePayloads {
                selector: id_selector("missing.component"),
                include_active_runtime_serialized_stores: true,
            }],
        )
        .unwrap_err();
    assert_eq!(error.field(), "operation.invalidate_payloads");
    assert_eq!(state, before_failure);
}

#[test]
fn beginning_runtime_lifetime_rekeys_owned_stores_and_preserves_physical_images() {
    let mut source = snapshot();
    for component in &mut source.environment.components {
        if matches!(component.binding, ComponentBinding::Session { .. }) {
            component.serialization_owner = SerializationOwner::None;
            component.lifetime = SemanticLifetime::Session;
        }
    }
    let session_before = source
        .environment
        .components
        .iter()
        .find(|component| component.id == "flow.main")
        .unwrap()
        .clone();
    let mut state = PlannerExecutionState::new(source).unwrap();
    state
        .apply_operations(
            "boundary.seed-physical-image",
            "snapshot.physical-image-seeded",
            &[StateOperation::SaveRuntimeToSlot {
                source_runtime_file_id: "file-0".into(),
                destination_slot: PhysicalSlotId(1),
                destination_persistent_file_id: "persistent-slot-1".into(),
                runtime_component_ids: vec!["raw.flags".into(), "save.main".into()],
                stage_bank_stages: Vec::new(),
            }],
        )
        .unwrap();
    let physical_image_before = state.persistent_file_images["persistent-slot-1"].clone();

    state.snapshot.environment.active_runtime_file = RuntimeFile {
        id: "loaded-a".into(),
        origin: RuntimeFileOrigin::LoadedSlot {
            slot: PhysicalSlotId(1),
        },
        backing: BackingAttachment::CardBacked {
            slot: PhysicalSlotId(1),
        },
        allowed_serialization_targets: vec![PhysicalSlotId(1)],
        lifecycle: RuntimeFileLifecycle::Active,
    };
    for component in &mut state.snapshot.environment.components {
        rekey_component_runtime(component, "file-0", "loaded-a");
        rekey_serialization_owner_runtime(&mut component.serialization_owner, "file-0", "loaded-a");
    }
    let stage_owner = SerializationOwner::StageBank {
        runtime_file_id: "loaded-a".into(),
        stage: "F_SP103".into(),
    };
    let mut stage_component = raw_component();
    stage_component.id = "stage.live".into();
    stage_component.binding = ComponentBinding::Stage {
        stage: "F_SP103".into(),
    };
    stage_component.lifetime = SemanticLifetime::StageLoad;
    stage_component.serialization_owner = stage_owner.clone();
    state
        .snapshot
        .environment
        .components
        .push(stage_component.clone());
    state
        .snapshot
        .environment
        .components
        .sort_by(|left, right| left.id.cmp(&right.id));
    state
        .serialized_components
        .insert(stage_owner, vec![stage_component]);
    let unrelated_owner = SerializationOwner::StageBank {
        runtime_file_id: "suspended".into(),
        stage: "D_MN05".into(),
    };
    let mut unrelated_component = raw_component();
    unrelated_component.id = "stage.unrelated".into();
    unrelated_component.binding = ComponentBinding::Stage {
        stage: "D_MN05".into(),
    };
    unrelated_component.lifetime = SemanticLifetime::StageLoad;
    unrelated_component.serialization_owner = unrelated_owner.clone();
    state
        .serialized_components
        .insert(unrelated_owner.clone(), vec![unrelated_component]);
    state
        .snapshot
        .environment
        .inactive_runtime_files
        .push(RuntimeFile {
            id: "suspended".into(),
            origin: RuntimeFileOrigin::NewFile,
            backing: BackingAttachment::MemoryOnly,
            allowed_serialization_targets: Vec::new(),
            lifecycle: RuntimeFileLifecycle::Suspended,
        });
    state.validate().unwrap();

    let mut colliding = state.clone();
    let colliding_runtime = RuntimeFile {
        id: "loaded-a.title-file-0".into(),
        origin: RuntimeFileOrigin::TitleFile0,
        backing: BackingAttachment::MemoryOnly,
        allowed_serialization_targets: Vec::new(),
        lifecycle: RuntimeFileLifecycle::Ended,
    };
    let insert_at = colliding
        .snapshot
        .environment
        .inactive_runtime_files
        .binary_search_by(|runtime| runtime.id.cmp(&colliding_runtime.id))
        .unwrap_err();
    colliding
        .snapshot
        .environment
        .inactive_runtime_files
        .insert(insert_at, colliding_runtime);
    colliding.validate().unwrap();
    let before_collision = colliding.clone();
    let error = colliding
        .apply_operations(
            "boundary.colliding-title-file-0",
            "snapshot.not-produced",
            &[StateOperation::BeginRuntimeFileLifetime {
                destination_id_suffix: "title-file-0".into(),
                origin: RuntimeFileOrigin::TitleFile0,
                backing: BackingAttachment::MemoryOnly,
                allowed_serialization_targets: Vec::new(),
            }],
        )
        .unwrap_err();
    assert_eq!(
        error.field(),
        "operation.begin_runtime_file_lifetime.destination_runtime_file_id"
    );
    assert_eq!(colliding, before_collision);

    state
        .apply_operations(
            "boundary.begin-title-file-0",
            "snapshot.title-file-0-active",
            &[StateOperation::BeginRuntimeFileLifetime {
                destination_id_suffix: "title-file-0".into(),
                origin: RuntimeFileOrigin::TitleFile0,
                backing: BackingAttachment::MemoryOnly,
                allowed_serialization_targets: vec![
                    PhysicalSlotId(1),
                    PhysicalSlotId(2),
                    PhysicalSlotId(3),
                ],
            }],
        )
        .unwrap();

    let destination = "loaded-a.title-file-0";
    assert_eq!(
        state.snapshot.environment.active_runtime_file.id,
        destination
    );
    assert_eq!(
        state.snapshot.environment.active_runtime_file.origin,
        RuntimeFileOrigin::TitleFile0
    );
    assert_eq!(
        state.snapshot.environment.active_runtime_file.backing,
        BackingAttachment::MemoryOnly
    );
    assert_eq!(
        state
            .snapshot
            .environment
            .inactive_runtime_files
            .iter()
            .find(|runtime| runtime.id == "loaded-a")
            .unwrap()
            .lifecycle,
        RuntimeFileLifecycle::Ended
    );
    let save = state
        .snapshot
        .environment
        .components
        .iter()
        .find(|component| component.id == "save.main")
        .unwrap();
    assert_eq!(
        save.binding,
        ComponentBinding::RuntimeFile {
            runtime_file_id: destination.into()
        }
    );
    assert_eq!(
        save.serialization_owner,
        SerializationOwner::RuntimeFile {
            runtime_file_id: destination.into()
        }
    );
    let destination_stage_owner = SerializationOwner::StageBank {
        runtime_file_id: destination.into(),
        stage: "F_SP103".into(),
    };
    assert!(
        state
            .serialized_components
            .contains_key(&destination_stage_owner)
    );
    assert!(state.serialized_components.contains_key(&unrelated_owner));
    assert_eq!(
        state
            .snapshot
            .environment
            .components
            .iter()
            .find(|component| component.id == "flow.main")
            .unwrap(),
        &session_before
    );
    assert_eq!(
        state.persistent_file_images["persistent-slot-1"],
        physical_image_before
    );
}

#[test]
fn search_identity_includes_non_snapshot_backing_stores() {
    let state = PlannerExecutionState::new(snapshot()).unwrap();
    let mut gated = state.clone();
    gated.gate_states.insert("gate.no-teleport".into(), true);
    let mut cleanup = state.clone();
    cleanup
        .scheduled_cleanup_ids
        .insert("cleanup.item-handoff".into());
    assert_ne!(state.digest().unwrap(), gated.digest().unwrap());
    assert_ne!(state.digest().unwrap(), cleanup.digest().unwrap());

    let mut history_only = state.clone();
    history_only.snapshot.id = "snapshot.other-history".into();
    history_only.snapshot.sequence = 99;
    mark_transition(
        &mut history_only.snapshot.environment.components[0],
        "transition.history-only",
    );
    assert_ne!(state.digest().unwrap(), history_only.digest().unwrap());
    assert_eq!(
        state.semantic_digest().unwrap(),
        history_only.semantic_digest().unwrap()
    );

    let document = state.to_document().unwrap();
    let bytes = document.canonical_bytes().unwrap();
    let decoded = PlannerExecutionStateDocument::decode_canonical(&bytes).unwrap();
    assert_eq!(decoded.into_state().unwrap(), state);
}

#[test]
fn boundary_policy_clears_unmentioned_components_and_honors_one_shot_preserve() {
    let mut state = PlannerExecutionState::new(snapshot()).unwrap();
    state
        .apply_operations(
            "technique.preserve-save",
            "snapshot.preserve-armed",
            &[StateOperation::Preserve {
                selector: id_selector("save.main"),
            }],
        )
        .unwrap();
    let policy = BoundaryPolicy {
        schema: BOUNDARY_POLICY_SCHEMA.into(),
        id: "boundary.room-load".into(),
        boundary: BoundaryKind::RoomTransition,
        default_disposition: BoundaryDisposition::Clear,
        component_rules: vec![ComponentBoundaryRule {
            selector: id_selector("flow.main"),
            disposition: BoundaryDisposition::Preserve,
        }],
    };
    state
        .apply_boundary(
            "boundary.room-load",
            "snapshot.after-room-load",
            &policy,
            &BTreeMap::new(),
        )
        .unwrap();
    let ids = state
        .snapshot
        .environment
        .components
        .iter()
        .map(|component| component.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, vec!["flow.main", "save.main"]);
    assert!(state.preserved_component_ids.is_empty());
}

#[test]
fn unknown_boundary_behavior_fails_atomically() {
    let mut state = PlannerExecutionState::new(snapshot()).unwrap();
    let before = state.clone();
    let policy = BoundaryPolicy {
        schema: BOUNDARY_POLICY_SCHEMA.into(),
        id: "boundary.unknown".into(),
        boundary: BoundaryKind::WrongStateRespawn,
        default_disposition: BoundaryDisposition::Unknown,
        component_rules: Vec::new(),
    };
    let error = state
        .apply_boundary(
            "boundary.unknown",
            "snapshot.not-produced",
            &policy,
            &BTreeMap::new(),
        )
        .unwrap_err();
    assert_eq!(error.field(), "boundary.disposition");
    assert_eq!(state, before);
}

#[test]
fn boundary_serialization_moves_selected_state_into_its_owner_store() {
    let mut state = PlannerExecutionState::new(snapshot()).unwrap();
    let owner = SerializationOwner::PhysicalSlot {
        slot: PhysicalSlotId(1),
    };
    let policy = BoundaryPolicy {
        schema: BOUNDARY_POLICY_SCHEMA.into(),
        id: "boundary.title-return".into(),
        boundary: BoundaryKind::TitleReturn,
        default_disposition: BoundaryDisposition::Clear,
        component_rules: vec![ComponentBoundaryRule {
            selector: id_selector("save.main"),
            disposition: BoundaryDisposition::Serialize {
                owner: owner.clone(),
            },
        }],
    };
    state
        .apply_boundary(
            "boundary.title-return",
            "snapshot.at-title",
            &policy,
            &BTreeMap::new(),
        )
        .unwrap();
    assert!(state.snapshot.environment.components.is_empty());
    assert_eq!(state.serialized_components[&owner][0].id, "save.main");
}

#[test]
fn raw_writes_and_invalidation_change_only_selected_knownness_bits() {
    let mut state = PlannerExecutionState::new(snapshot()).unwrap();
    state
        .apply_operations(
            "transition.consume-key",
            "snapshot.after-key",
            &[
                StateOperation::WriteRaw {
                    component_id: "raw.flags".into(),
                    byte_offset: 0,
                    mask: vec![0x30],
                    value: vec![0x30],
                },
                StateOperation::InvalidateRaw {
                    component_id: "raw.flags".into(),
                    byte_offset: 0,
                    mask: vec![0x10],
                },
                StateOperation::Write {
                    target: ComponentFieldTarget {
                        component_id: "save.main".into(),
                        field: "small_keys".into(),
                    },
                    value: StateValue::Unsigned(2),
                },
                StateOperation::Adjust {
                    target: ComponentFieldTarget {
                        component_id: "save.main".into(),
                        field: "small_keys".into(),
                    },
                    delta: -1,
                },
            ],
        )
        .unwrap();
    assert_eq!(
        field(&state, "save.main", "small_keys"),
        &StateValue::Unsigned(1)
    );
    let raw = state
        .snapshot
        .environment
        .components
        .iter()
        .find(|component| component.id == "raw.flags")
        .unwrap();
    assert_eq!(
        raw.payload,
        ComponentPayload::Raw {
            bytes: vec![0x30],
            known_mask: vec![0x20]
        }
    );
}

#[test]
fn bound_raw_writes_follow_current_stage_and_fail_atomically_on_ambiguity() {
    let mut state = PlannerExecutionState::new(snapshot()).unwrap();
    state.snapshot.environment.location.stage = "D_MN05".into();
    let mut component = raw_component();
    component.id = "stage.raw-flags".into();
    component.component_kind = ComponentKind::StageMemory;
    component.binding = ComponentBinding::Stage {
        stage: "D_MN05".into(),
    };
    component.lifetime = SemanticLifetime::StageLoad;
    component.serialization_owner = SerializationOwner::StageBank {
        runtime_file_id: "file-0".into(),
        stage: "D_MN05".into(),
    };
    state.snapshot.environment.components.push(component);
    state
        .snapshot
        .environment
        .components
        .sort_by(|left, right| left.id.cmp(&right.id));
    state.validate().unwrap();

    state
        .apply_operations(
            "transition.write-stage-switch",
            "snapshot.stage-switch-written",
            &[
                StateOperation::WriteBoundRaw {
                    component_kind: ComponentKind::StageMemory,
                    binding: ComponentBindingReference::CurrentStage,
                    byte_offset: 0,
                    mask: vec![0x30],
                    value: vec![0x20],
                },
                StateOperation::InvalidateBoundRaw {
                    component_kind: ComponentKind::StageMemory,
                    binding: ComponentBindingReference::CurrentStage,
                    byte_offset: 0,
                    mask: vec![0x10],
                },
            ],
        )
        .unwrap();
    let component = state
        .snapshot
        .environment
        .components
        .iter()
        .find(|component| component.id == "stage.raw-flags")
        .unwrap();
    assert_eq!(
        component.payload,
        ComponentPayload::Raw {
            bytes: vec![0x20],
            known_mask: vec![0x20],
        }
    );

    let mut duplicate = component.clone();
    duplicate.id = "stage.raw-flags.duplicate".into();
    state.snapshot.environment.components.push(duplicate);
    state
        .snapshot
        .environment
        .components
        .sort_by(|left, right| left.id.cmp(&right.id));
    let before = state.clone();
    assert!(
        state
            .apply_operations(
                "transition.ambiguous-stage-switch",
                "snapshot.not-produced",
                &[StateOperation::WriteBoundRaw {
                    component_kind: ComponentKind::StageMemory,
                    binding: ComponentBindingReference::CurrentStage,
                    byte_offset: 0,
                    mask: vec![1],
                    value: vec![1],
                }],
            )
            .is_err()
    );
    assert_eq!(state, before);
}

#[test]
fn bound_raw_writes_follow_a_binding_projected_from_live_flow_state() {
    let mut snapshot = snapshot();
    snapshot.environment.components.extend([
        StateComponent {
            id: "message-session".into(),
            component_kind: ComponentKind::MessageFlow,
            payload: ComponentPayload::Structured {
                fields: BTreeMap::from([
                    ("speaker_stage".into(), StateValue::Text("D_MN01".into())),
                    ("speaker_zone".into(), StateValue::Signed(7)),
                ]),
            },
            binding: ComponentBinding::Global,
            lifetime: SemanticLifetime::Action,
            serialization_owner: SerializationOwner::None,
            provenance: provenance(),
        },
        StateComponent {
            id: "zone.raw".into(),
            component_kind: ComponentKind::ZoneMemory,
            payload: ComponentPayload::Raw {
                bytes: vec![0],
                known_mask: vec![0xff],
            },
            binding: ComponentBinding::Zone {
                stage: "D_MN01".into(),
                zone: 7,
            },
            lifetime: SemanticLifetime::RoomLoad,
            serialization_owner: SerializationOwner::None,
            provenance: provenance(),
        },
    ]);
    snapshot
        .environment
        .components
        .sort_by(|left, right| left.id.cmp(&right.id));
    let mut state = PlannerExecutionState::new(snapshot).unwrap();
    let operation = StateOperation::WriteBoundRaw {
        component_kind: ComponentKind::ZoneMemory,
        binding: ComponentBindingReference::Projected {
            component_id: "message-session".into(),
            projection: Box::new(ComponentBindingProjection::Zone {
                stage_field: "speaker_stage".into(),
                zone_field: "speaker_zone".into(),
            }),
        },
        byte_offset: 0,
        mask: vec![0x20],
        value: vec![0x20],
    };
    state
        .apply_operations(
            "transition.message-zone-write",
            "snapshot.message-zone-written",
            std::slice::from_ref(&operation),
        )
        .unwrap();
    let zone = state
        .snapshot
        .environment
        .components
        .iter()
        .find(|component| component.id == "zone.raw")
        .unwrap();
    assert!(matches!(
        &zone.payload,
        ComponentPayload::Raw { bytes, .. } if bytes == &[0x20]
    ));

    let flow = state
        .snapshot
        .environment
        .components
        .iter_mut()
        .find(|component| component.id == "message-session")
        .unwrap();
    flow.payload = ComponentPayload::Unknown {
        expected_bytes: None,
    };
    let before = state.clone();
    assert!(
        state
            .apply_operations(
                "transition.unresolved-message-zone-write",
                "snapshot.not-produced",
                &[operation],
            )
            .is_err()
    );
    assert_eq!(state, before);
}

#[test]
fn bound_raw_unsigned_adjusts_only_one_known_stage_bank_atomically() {
    let mut state = PlannerExecutionState::new(snapshot()).unwrap();
    state.snapshot.environment.location.stage = "D_MN05".into();
    let mut bytes = vec![0_u8; 0x20];
    bytes[0x1c] = 2;
    state.snapshot.environment.components.push(StateComponent {
        id: "stage-memory.active".into(),
        component_kind: ComponentKind::DungeonMemory,
        payload: ComponentPayload::Raw {
            bytes,
            known_mask: vec![0xff; 0x20],
        },
        binding: ComponentBinding::Stage {
            stage: "D_MN05".into(),
        },
        lifetime: SemanticLifetime::StageLoad,
        serialization_owner: SerializationOwner::StageBank {
            runtime_file_id: "file-0".into(),
            stage: "D_MN05".into(),
        },
        provenance: provenance(),
    });
    state
        .snapshot
        .environment
        .components
        .sort_by(|left, right| left.id.cmp(&right.id));
    state.validate().unwrap();

    let adjust = |delta| StateOperation::AdjustBoundRawUnsigned {
        component_kind: ComponentKind::DungeonMemory,
        binding: ComponentBindingReference::CurrentStage,
        byte_offset: 0x1c,
        byte_width: 1,
        delta,
    };
    state
        .apply_operations("pickup.small-key", "snapshot.three-keys", &[adjust(1)])
        .unwrap();
    state
        .apply_operations("door.consume-key", "snapshot.two-keys", &[adjust(-1)])
        .unwrap();
    let stage_memory = state
        .snapshot
        .environment
        .components
        .iter()
        .find(|component| component.id == "stage-memory.active")
        .unwrap();
    let ComponentPayload::Raw { bytes, .. } = &stage_memory.payload else {
        unreachable!()
    };
    assert_eq!(bytes[0x1c], 2);
    assert!(bytes[..0x1c].iter().all(|byte| *byte == 0));
    assert!(bytes[0x1d..].iter().all(|byte| *byte == 0));
    assert_eq!(
        state.execution_history.last().unwrap().event,
        ExecutionHistoryKind::Operation {
            operation: adjust(-1),
            affected_component_ids: vec!["stage-memory.active".into()],
        }
    );

    let before_failure = state.clone();
    let error = state
        .apply_operations(
            "door.wrong-bank",
            "snapshot.not-produced",
            &[StateOperation::AdjustBoundRawUnsigned {
                component_kind: ComponentKind::DungeonMemory,
                binding: ComponentBindingReference::Exact {
                    binding: ComponentBinding::Stage {
                        stage: "D_MN04".into(),
                    },
                },
                byte_offset: 0x1c,
                byte_width: 1,
                delta: -1,
            }],
        )
        .unwrap_err();
    assert_eq!(error.field(), "operation.adjust_bound_raw_unsigned");
    assert_eq!(state, before_failure);

    let before_underflow = state.clone();
    assert!(
        state
            .apply_operations(
                "door.consume-too-many-keys",
                "snapshot.no-underflow",
                &[adjust(-3)],
            )
            .is_err()
    );
    assert_eq!(state, before_underflow);

    let mut duplicate = state
        .snapshot
        .environment
        .components
        .iter()
        .find(|component| component.id == "stage-memory.active")
        .unwrap()
        .clone();
    duplicate.id = "stage-memory.ambiguous".into();
    state.snapshot.environment.components.push(duplicate);
    state
        .snapshot
        .environment
        .components
        .sort_by(|left, right| left.id.cmp(&right.id));
    let before_ambiguity = state.clone();
    assert!(
        state
            .apply_operations(
                "door.ambiguous-key-bank",
                "snapshot.no-ambiguous-write",
                &[adjust(-1)],
            )
            .is_err()
    );
    assert_eq!(state, before_ambiguity);
    state
        .snapshot
        .environment
        .components
        .retain(|component| component.id != "stage-memory.ambiguous");

    let stage_memory = state
        .snapshot
        .environment
        .components
        .iter_mut()
        .find(|component| component.id == "stage-memory.active")
        .unwrap();
    let ComponentPayload::Raw { known_mask, .. } = &mut stage_memory.payload else {
        unreachable!()
    };
    known_mask[0x1c] = 0;
    let before_unknown = state.clone();
    assert!(
        state
            .apply_operations(
                "door.unknown-key-count",
                "snapshot.not-produced-either",
                &[adjust(-1)],
            )
            .is_err()
    );
    assert_eq!(state, before_unknown);
}

#[test]
fn serialization_clear_and_restore_keep_the_owner_store_independent() {
    let mut state = PlannerExecutionState::new(snapshot()).unwrap();
    let owner = SerializationOwner::PhysicalSlot {
        slot: PhysicalSlotId(1),
    };
    state
        .apply_operations(
            "transition.save-load",
            "snapshot.restored",
            &[
                StateOperation::Serialize {
                    selector: id_selector("save.main"),
                    owner: owner.clone(),
                },
                StateOperation::ClearComponent {
                    selector: id_selector("save.main"),
                },
                StateOperation::Restore {
                    owner: owner.clone(),
                    destination_component_id: "save.main".into(),
                },
            ],
        )
        .unwrap();
    assert_eq!(state.serialized_components[&owner].len(), 1);
    assert!(
        state
            .snapshot
            .environment
            .components
            .iter()
            .any(|component| component.id == "save.main")
    );
    assert_eq!(
        state.serialized_components[&owner][0].serialization_owner,
        owner
    );
}

#[test]
fn serialized_store_keys_and_stage_bank_lifetimes_are_enforced() {
    let mut mismatched_owner = PlannerExecutionState::new(snapshot()).unwrap();
    mismatched_owner.serialized_components.insert(
        SerializationOwner::PhysicalSlot {
            slot: PhysicalSlotId(1),
        },
        vec![structured_component(
            "stored.save",
            ComponentKind::PersistentSave,
            ComponentBinding::RuntimeFile {
                runtime_file_id: "file-0".into(),
            },
        )],
    );
    assert_eq!(
        mismatched_owner.validate().unwrap_err().field(),
        "serialized_components.owner"
    );

    let mut wrong_lifetime = PlannerExecutionState::new(snapshot()).unwrap();
    let owner = SerializationOwner::StageBank {
        runtime_file_id: "file-0".into(),
        stage: "F_SP103".into(),
    };
    let mut component = structured_component(
        "stage.stored",
        ComponentKind::StageMemory,
        ComponentBinding::Stage {
            stage: "F_SP103".into(),
        },
    );
    component.serialization_owner = owner.clone();
    wrong_lifetime
        .serialized_components
        .insert(owner, vec![component]);
    assert_eq!(
        wrong_lifetime.validate().unwrap_err().field(),
        "serialized_components.stage_bank"
    );
}

#[test]
fn normal_stage_bank_commit_load_is_runtime_scoped_and_atomic() {
    let stage_component = |stage: &str, marker: u64| {
        let mut component = structured_component(
            "stage.live",
            ComponentKind::StageMemory,
            ComponentBinding::Stage {
                stage: stage.into(),
            },
        );
        component.lifetime = SemanticLifetime::StageLoad;
        component.serialization_owner = SerializationOwner::StageBank {
            runtime_file_id: "file-0".into(),
            stage: stage.into(),
        };
        let ComponentPayload::Structured { fields } = &mut component.payload else {
            unreachable!()
        };
        fields.insert("marker".into(), StateValue::Unsigned(marker));
        component
    };
    let mut source = snapshot();
    source
        .environment
        .components
        .push(stage_component("F_SP103", 11));
    source
        .environment
        .components
        .sort_by(|left, right| left.id.cmp(&right.id));
    let mut state = PlannerExecutionState::new(source).unwrap();
    let destination_owner = SerializationOwner::StageBank {
        runtime_file_id: "file-0".into(),
        stage: "D_MN05".into(),
    };
    state.serialized_components.insert(
        destination_owner.clone(),
        vec![stage_component("D_MN05", 22)],
    );
    let other_file_owner = SerializationOwner::StageBank {
        runtime_file_id: "file-1".into(),
        stage: "D_MN05".into(),
    };
    let mut other_file = stage_component("D_MN05", 99);
    other_file.serialization_owner = other_file_owner.clone();
    state
        .serialized_components
        .insert(other_file_owner, vec![other_file]);
    state.validate().unwrap();

    state
        .apply_operations(
            "boundary.faron-to-forest",
            "snapshot.forest-bank-loaded",
            &[
                StateOperation::CommitLoadStageBank {
                    component_id: "stage.live".into(),
                    runtime_file_id: "file-0".into(),
                    source_stage: "F_SP103".into(),
                    destination_stage: "D_MN05".into(),
                    source_binding: ComponentBinding::Stage {
                        stage: "F_SP103".into(),
                    },
                    destination_binding: ComponentBinding::Stage {
                        stage: "D_MN05".into(),
                    },
                },
                StateOperation::SetLocation {
                    location: SceneLocation {
                        stage: "D_MN05".into(),
                        room: 0,
                        layer: 0,
                        spawn: 0,
                    },
                },
            ],
        )
        .unwrap();
    assert_eq!(
        field(&state, "stage.live", "marker"),
        &StateValue::Unsigned(22)
    );
    let source_owner = SerializationOwner::StageBank {
        runtime_file_id: "file-0".into(),
        stage: "F_SP103".into(),
    };
    let ComponentPayload::Structured { fields } =
        &state.serialized_components[&source_owner][0].payload
    else {
        unreachable!()
    };
    assert_eq!(fields["marker"], StateValue::Unsigned(11));
    let other_file_owner = SerializationOwner::StageBank {
        runtime_file_id: "file-1".into(),
        stage: "D_MN05".into(),
    };
    let ComponentPayload::Structured { fields } =
        &state.serialized_components[&other_file_owner][0].payload
    else {
        unreachable!()
    };
    assert_eq!(fields["marker"], StateValue::Unsigned(99));
    assert_eq!(state.snapshot.environment.location.stage, "D_MN05");
    assert_eq!(
        state
            .snapshot
            .environment
            .components
            .iter()
            .find(|component| component.id == "stage.live")
            .unwrap()
            .serialization_owner,
        destination_owner
    );

    let before = state.clone();
    let error = state
        .apply_operations(
            "boundary.wrong-file",
            "snapshot.not-produced",
            &[StateOperation::CommitLoadStageBank {
                component_id: "stage.live".into(),
                runtime_file_id: "file-1".into(),
                source_stage: "D_MN05".into(),
                destination_stage: "F_SP103".into(),
                source_binding: ComponentBinding::Stage {
                    stage: "D_MN05".into(),
                },
                destination_binding: ComponentBinding::Stage {
                    stage: "F_SP103".into(),
                },
            }],
        )
        .unwrap_err();
    assert_eq!(
        error.field(),
        "operation.commit_load_stage_bank.runtime_file_id"
    );
    assert_eq!(state, before);

    let before = state.clone();
    let error = state
        .apply_operations(
            "boundary.missing-destination",
            "snapshot.not-produced",
            &[StateOperation::CommitLoadStageBank {
                component_id: "stage.live".into(),
                runtime_file_id: "file-0".into(),
                source_stage: "D_MN05".into(),
                destination_stage: "D_MN06".into(),
                source_binding: ComponentBinding::Stage {
                    stage: "D_MN05".into(),
                },
                destination_binding: ComponentBinding::Stage {
                    stage: "D_MN06".into(),
                },
            }],
        )
        .unwrap_err();
    assert_eq!(
        error.field(),
        "operation.commit_load_stage_bank.destination"
    );
    assert_eq!(state, before);
}

#[test]
fn file_zero_save_and_load_preserve_nested_stores_and_end_only_the_runtime_lifetime() {
    let stage_component = |stage: &str, marker: u64| {
        let mut component = structured_component(
            "stage.live",
            ComponentKind::StageMemory,
            ComponentBinding::Stage {
                stage: stage.into(),
            },
        );
        component.lifetime = SemanticLifetime::StageLoad;
        component.serialization_owner = SerializationOwner::StageBank {
            runtime_file_id: "file-0".into(),
            stage: stage.into(),
        };
        let ComponentPayload::Structured { fields } = &mut component.payload else {
            unreachable!()
        };
        fields.insert("marker".into(), StateValue::Unsigned(marker));
        component
    };
    let mut source = snapshot();
    let save = source
        .environment
        .components
        .iter_mut()
        .find(|component| component.id == "save.main")
        .unwrap();
    let ComponentPayload::Structured { fields } = &mut save.payload else {
        unreachable!()
    };
    fields.insert("rupees".into(), StateValue::Unsigned(99));
    let mut raw_counter = raw_component();
    raw_counter.id = "save.raw-counter".into();
    raw_counter.binding = ComponentBinding::RuntimeFile {
        runtime_file_id: "file-0".into(),
    };
    let ComponentPayload::Raw { bytes, known_mask } = &mut raw_counter.payload else {
        unreachable!()
    };
    bytes[0] = 4;
    known_mask[0] = 0xff;
    source.environment.components.push(raw_counter);
    source
        .environment
        .components
        .push(stage_component("F_SP103", 11));
    let mut session = structured_component(
        "session.recent-item",
        ComponentKind::Session,
        ComponentBinding::Session {
            session_id: "process".into(),
        },
    );
    session.lifetime = SemanticLifetime::Session;
    session.serialization_owner = SerializationOwner::None;
    let ComponentPayload::Structured { fields } = &mut session.payload else {
        unreachable!()
    };
    fields.insert("item".into(), StateValue::Unsigned(0x4a));
    source.environment.components.push(session);
    let mut carried = structured_component(
        "runtime.bite-equipment",
        ComponentKind::Custom {
            id: "bite-equipment-transfer".into(),
        },
        ComponentBinding::RuntimeFile {
            runtime_file_id: "file-0".into(),
        },
    );
    let ComponentPayload::Structured { fields } = &mut carried.payload else {
        unreachable!()
    };
    fields.insert("equipped_item".into(), StateValue::Unsigned(0x28));
    source.environment.components.push(carried);
    source.environment.components.push(structured_component(
        "runtime.unselected-metadata",
        ComponentKind::Custom {
            id: "unselected-runtime-metadata".into(),
        },
        ComponentBinding::RuntimeFile {
            runtime_file_id: "file-0".into(),
        },
    ));
    source
        .environment
        .components
        .sort_by(|left, right| left.id.cmp(&right.id));
    let mut state = PlannerExecutionState::new(source).unwrap();
    for (stage, marker) in [("D_MN05", 22), ("F_SP103", 1)] {
        let owner = SerializationOwner::StageBank {
            runtime_file_id: "file-0".into(),
            stage: stage.into(),
        };
        let mut component = stage_component(stage, marker);
        component.serialization_owner = owner.clone();
        state.serialized_components.insert(owner, vec![component]);
    }
    state.validate().unwrap();

    let save_operation = StateOperation::SaveRuntimeToSlot {
        source_runtime_file_id: "file-0".into(),
        destination_slot: PhysicalSlotId(1),
        destination_persistent_file_id: "persistent-slot-1".into(),
        runtime_component_ids: vec!["save.main".into(), "save.raw-counter".into()],
        stage_bank_stages: vec!["D_MN05".into(), "F_SP103".into()],
    };
    state
        .apply_operations(
            "boundary.save-file-0",
            "snapshot.file-0-saved",
            &[save_operation],
        )
        .unwrap();
    assert_eq!(
        state.snapshot.environment.active_runtime_file.origin,
        RuntimeFileOrigin::TitleFile0
    );
    assert_eq!(state.snapshot.environment.physical_slots.len(), 1);
    let image = &state.persistent_file_images["persistent-slot-1"];
    assert_eq!(image.stage_banks.len(), 2);
    let saved_faron = image
        .stage_banks
        .iter()
        .find(|store| {
            matches!(
                &store.owner,
                SerializationOwner::StageBank { stage, .. } if stage == "F_SP103"
            )
        })
        .unwrap();
    let ComponentPayload::Structured { fields } = &saved_faron.components[0].payload else {
        unreachable!()
    };
    assert_eq!(fields["marker"], StateValue::Unsigned(11));
    assert_eq!(
        image.runtime_components[0]
            .provenance
            .last()
            .unwrap()
            .source_kind,
        ProvenanceSourceKind::SaveRestore
    );

    let before_failed_load = state.clone();
    let error = state
        .apply_operations(
            "boundary.incomplete-load",
            "snapshot.not-produced",
            &[StateOperation::LoadRuntimeFromSlot {
                source_runtime_file_id: "file-0".into(),
                source_slot: PhysicalSlotId(1),
                source_persistent_file_id: "persistent-slot-1".into(),
                destination_runtime_file_id: "slot-1-runtime".into(),
                destination_allowed_serialization_targets: vec![PhysicalSlotId(1)],
                runtime_component_ids: vec!["save.main".into(), "save.raw-counter".into()],
                stage_bank_stages: vec!["F_SP103".into()],
                carried_runtime_component_ids: Vec::new(),
            }],
        )
        .unwrap_err();
    assert_eq!(
        error.field(),
        "operation.load_runtime_from_slot.stage_bank_stages"
    );
    assert_eq!(state, before_failed_load);

    let before_failed_carry = state.clone();
    let error = state
        .apply_operations(
            "boundary.invalid-runtime-carry",
            "snapshot.not-produced",
            &[StateOperation::LoadRuntimeFromSlot {
                source_runtime_file_id: "file-0".into(),
                source_slot: PhysicalSlotId(1),
                source_persistent_file_id: "persistent-slot-1".into(),
                destination_runtime_file_id: "slot-1-runtime".into(),
                destination_allowed_serialization_targets: vec![PhysicalSlotId(1)],
                runtime_component_ids: vec!["save.main".into(), "save.raw-counter".into()],
                stage_bank_stages: vec!["D_MN05".into(), "F_SP103".into()],
                carried_runtime_component_ids: vec!["session.recent-item".into()],
            }],
        )
        .unwrap_err();
    assert_eq!(
        error.field(),
        "operation.load_runtime_from_slot.carried_runtime_component_ids"
    );
    assert_eq!(state, before_failed_carry);

    let mut dynamic_load = state.clone();
    let sealed_image_before = dynamic_load.persistent_file_images["persistent-slot-1"].clone();
    dynamic_load
        .apply_operations(
            "boundary.dynamic-load-slot-1",
            "snapshot.dynamic-slot-1-loaded",
            &[StateOperation::LoadActiveRuntimeFromSlot {
                source_slot: PhysicalSlotId(1),
                destination_id_suffix: "file-select-slot-1".into(),
                destination_allowed_serialization_targets: vec![
                    PhysicalSlotId(1),
                    PhysicalSlotId(2),
                    PhysicalSlotId(3),
                ],
                carried_runtime_component_ids: vec!["runtime.bite-equipment".into()],
            }],
        )
        .unwrap();
    assert_eq!(
        dynamic_load.snapshot.environment.active_runtime_file.id,
        "file-0.file-select-slot-1"
    );
    assert_eq!(
        dynamic_load.snapshot.environment.active_runtime_file.origin,
        RuntimeFileOrigin::LoadedSlot {
            slot: PhysicalSlotId(1)
        }
    );
    assert_eq!(
        field(&dynamic_load, "save.main", "rupees"),
        &StateValue::Unsigned(99)
    );
    assert_eq!(
        field(&dynamic_load, "runtime.bite-equipment", "equipped_item"),
        &StateValue::Unsigned(0x28)
    );
    assert!(
        dynamic_load
            .snapshot
            .environment
            .components
            .iter()
            .all(|component| component.id != "runtime.unselected-metadata")
    );
    assert_eq!(
        dynamic_load.persistent_file_images["persistent-slot-1"],
        sealed_image_before
    );

    state
        .apply_operations(
            "boundary.load-slot-1",
            "snapshot.slot-1-loaded",
            &[
                StateOperation::LoadRuntimeFromSlot {
                    source_runtime_file_id: "file-0".into(),
                    source_slot: PhysicalSlotId(1),
                    source_persistent_file_id: "persistent-slot-1".into(),
                    destination_runtime_file_id: "slot-1-runtime".into(),
                    destination_allowed_serialization_targets: vec![PhysicalSlotId(1)],
                    runtime_component_ids: vec!["save.main".into(), "save.raw-counter".into()],
                    stage_bank_stages: vec!["D_MN05".into(), "F_SP103".into()],
                    carried_runtime_component_ids: vec!["runtime.bite-equipment".into()],
                },
                StateOperation::ActivateStageBank {
                    component_id: "stage.live".into(),
                    runtime_file_id: "slot-1-runtime".into(),
                    stage: "F_SP103".into(),
                    binding: ComponentBinding::Stage {
                        stage: "F_SP103".into(),
                    },
                },
                StateOperation::SetLocation {
                    location: SceneLocation {
                        stage: "F_SP103".into(),
                        room: 0,
                        layer: 0,
                        spawn: 0,
                    },
                },
            ],
        )
        .unwrap();
    assert_eq!(
        state.snapshot.environment.active_runtime_file,
        RuntimeFile {
            id: "slot-1-runtime".into(),
            origin: RuntimeFileOrigin::LoadedSlot {
                slot: PhysicalSlotId(1)
            },
            backing: BackingAttachment::CardBacked {
                slot: PhysicalSlotId(1)
            },
            allowed_serialization_targets: vec![PhysicalSlotId(1)],
            lifecycle: RuntimeFileLifecycle::Active,
        }
    );
    assert_eq!(state.snapshot.environment.inactive_runtime_files.len(), 1);
    assert_eq!(
        state.snapshot.environment.inactive_runtime_files[0].lifecycle,
        RuntimeFileLifecycle::Ended
    );
    assert_eq!(
        field(&state, "save.main", "rupees"),
        &StateValue::Unsigned(99)
    );
    assert_eq!(
        field(&state, "stage.live", "marker"),
        &StateValue::Unsigned(11)
    );
    assert_eq!(
        field(&state, "session.recent-item", "item"),
        &StateValue::Unsigned(0x4a)
    );
    assert_eq!(
        field(&state, "runtime.bite-equipment", "equipped_item"),
        &StateValue::Unsigned(0x28)
    );
    let carried = state
        .snapshot
        .environment
        .components
        .iter()
        .find(|component| component.id == "runtime.bite-equipment")
        .unwrap();
    assert_eq!(
        carried.binding,
        ComponentBinding::RuntimeFile {
            runtime_file_id: "slot-1-runtime".into(),
        }
    );
    assert_eq!(
        carried.serialization_owner,
        SerializationOwner::RuntimeFile {
            runtime_file_id: "slot-1-runtime".into(),
        }
    );
    assert_eq!(
        carried.provenance.last().unwrap().transition_id.as_deref(),
        Some("boundary.load-slot-1")
    );
    assert!(
        state
            .snapshot
            .environment
            .components
            .iter()
            .all(|component| component.id != "runtime.unselected-metadata")
    );
    assert_eq!(
        state
            .snapshot
            .environment
            .components
            .iter()
            .find(|component| component.id == "save.raw-counter")
            .unwrap()
            .binding,
        ComponentBinding::RuntimeFile {
            runtime_file_id: "slot-1-runtime".into(),
        }
    );
    state
        .apply_operations(
            "transition.increment-loaded-counter",
            "snapshot.loaded-counter-incremented",
            &[StateOperation::AdjustBoundRawUnsigned {
                component_kind: ComponentKind::PersistentSave,
                binding: ComponentBindingReference::ActiveRuntimeFile,
                byte_offset: 0,
                byte_width: 1,
                delta: 1,
            }],
        )
        .unwrap();
    let loaded_counter = state
        .snapshot
        .environment
        .components
        .iter()
        .find(|component| component.id == "save.raw-counter")
        .unwrap();
    assert_eq!(
        loaded_counter.binding,
        ComponentBinding::RuntimeFile {
            runtime_file_id: "slot-1-runtime".into(),
        }
    );
    let ComponentPayload::Raw { bytes, .. } = &loaded_counter.payload else {
        unreachable!()
    };
    assert_eq!(bytes[0], 5);
    assert!(
        state
            .serialized_components
            .contains_key(&SerializationOwner::StageBank {
                runtime_file_id: "slot-1-runtime".into(),
                stage: "D_MN05".into(),
            })
    );
    assert!(
        !state
            .serialized_components
            .keys()
            .any(|owner| { owner_belongs_to_runtime(owner, "file-0") })
    );

    let sealed_slot_digest = state.snapshot.environment.physical_slots[0].serialized_state_sha256;
    state
        .apply_operations(
            "transition.spend-rupees",
            "snapshot.runtime-diverged-from-slot",
            &[StateOperation::Write {
                target: ComponentFieldTarget {
                    component_id: "save.main".into(),
                    field: "rupees".into(),
                },
                value: StateValue::Unsigned(1),
            }],
        )
        .unwrap();
    assert_eq!(
        field(&state, "save.main", "rupees"),
        &StateValue::Unsigned(1)
    );
    let ComponentPayload::Structured { fields } =
        &state.persistent_file_images["persistent-slot-1"].runtime_components[0].payload
    else {
        unreachable!()
    };
    assert_eq!(fields["rupees"], StateValue::Unsigned(99));
    assert_eq!(
        state.snapshot.environment.physical_slots[0].serialized_state_sha256,
        sealed_slot_digest
    );

    let document = state.to_document().unwrap();
    let decoded =
        PlannerExecutionStateDocument::decode_canonical(&document.canonical_bytes().unwrap())
            .unwrap()
            .into_state()
            .unwrap();
    assert_eq!(decoded, state);
    let semantic_with_history = state.semantic_digest().unwrap();
    let mut without_ended_history = state.clone();
    without_ended_history
        .snapshot
        .environment
        .inactive_runtime_files
        .clear();
    assert_eq!(
        semantic_with_history,
        without_ended_history.semantic_digest().unwrap()
    );
}

#[test]
fn active_runtime_save_derives_the_persistent_identity_at_execution_time() {
    let mut state = PlannerExecutionState::new(snapshot()).unwrap();
    state
        .apply_operations(
            "boundary.dynamic-save-slot-1",
            "snapshot.dynamic-save-complete",
            &[StateOperation::SaveActiveRuntimeToSlot {
                destination_slot: PhysicalSlotId(1),
                destination_id_suffix: "save-slot-1".into(),
                runtime_component_ids: vec!["raw.flags".into(), "save.main".into()],
                projection_operations: Vec::new(),
            }],
        )
        .unwrap();

    assert_eq!(
        state.snapshot.environment.physical_slots[0].persistent_file_id,
        "file-0.save-slot-1"
    );
    let image = &state.persistent_file_images["file-0.save-slot-1"];
    assert_eq!(image.source_runtime_file_id, "file-0");
    assert_eq!(
        image
            .runtime_components
            .iter()
            .map(|component| component.id.as_str())
            .collect::<Vec<_>>(),
        vec!["raw.flags", "save.main"]
    );
}

#[test]
fn copy_move_rebind_and_projection_transform_only_named_components() {
    let mut state = PlannerExecutionState::new(snapshot()).unwrap();
    state
        .apply_operations(
            "technique.component-transfer",
            "snapshot.transferred",
            &[
                StateOperation::Copy {
                    source: id_selector("save.main"),
                    destination_component_id: "save.copy".into(),
                    binding: ComponentBinding::Unbound,
                    serialization_owner: SerializationOwner::None,
                },
                StateOperation::Bind {
                    selector: id_selector("save.copy"),
                    binding: ComponentBinding::Dungeon {
                        dungeon: "forest".into(),
                    },
                },
                StateOperation::Move {
                    source: id_selector("save.copy"),
                    destination_component_id: "forest.memory".into(),
                    binding: ComponentBinding::Stage {
                        stage: "D_MN05".into(),
                    },
                    serialization_owner: SerializationOwner::StageBank {
                        runtime_file_id: "file-0".into(),
                        stage: "D_MN05".into(),
                    },
                },
                StateOperation::Rebind {
                    selector: id_selector("forest.memory"),
                    binding: ComponentBinding::Stage {
                        stage: "D_MN06".into(),
                    },
                },
                StateOperation::Project {
                    source_runtime_file_id: "file-0".into(),
                    destination_runtime_file_id: "file-1".into(),
                    component_ids: vec!["save.main".into()],
                },
            ],
        )
        .unwrap();

    let components = &state.snapshot.environment.components;
    assert!(
        !components
            .iter()
            .any(|component| component.id == "save.copy")
    );
    assert_eq!(
        components
            .iter()
            .find(|component| component.id == "forest.memory")
            .unwrap()
            .binding,
        ComponentBinding::Stage {
            stage: "D_MN06".into()
        }
    );
    let projected = components
        .iter()
        .find(|component| component.id == "save.main")
        .unwrap();
    assert_eq!(
        projected.binding,
        ComponentBinding::RuntimeFile {
            runtime_file_id: "file-1".into()
        }
    );
    assert_eq!(
        projected.serialization_owner,
        SerializationOwner::RuntimeFile {
            runtime_file_id: "file-1".into()
        }
    );
}

#[test]
fn message_and_pending_operation_state_is_not_collapsed_to_a_boolean() {
    let mut state = PlannerExecutionState::new(snapshot()).unwrap();
    state
        .apply_operations(
            "technique.dialogue-interrupt",
            "snapshot.dialogue-interrupted",
            &[
                StateOperation::ScheduleCleanup {
                    cleanup_id: "cleanup.item-handoff".into(),
                },
                StateOperation::BranchFlow {
                    flow_component_id: "flow.main".into(),
                    edge_id: "edge.reward".into(),
                    destination_node_id: "node.reward".into(),
                },
                StateOperation::Interrupt {
                    action_id: "action.sidehop".into(),
                    window: TemporalWindow {
                        earliest_frame: 14,
                        latest_frame: 14,
                        required_input: Some("input.sidehop".into()),
                    },
                },
                StateOperation::CancelCleanup {
                    cleanup_id: "cleanup.item-handoff".into(),
                },
                StateOperation::Consume {
                    pending_operation_id: "pending.item".into(),
                },
            ],
        )
        .unwrap();
    assert_eq!(
        field(&state, "flow.main", "node_id"),
        &StateValue::Text("node.reward".into())
    );
    assert_eq!(
        field(&state, "flow.main", "last_edge_id"),
        &StateValue::Text("edge.reward".into())
    );
    assert!(state.scheduled_cleanup_ids.is_empty());
    assert_eq!(state.interruption_log[0].window.earliest_frame, 14);
    assert!(
        !state
            .snapshot
            .environment
            .components
            .iter()
            .any(|component| component.id == "pending.item")
    );
}

#[test]
fn actor_reconstruction_joins_placement_layer_persistence_and_lifecycle_atomically() {
    let mut source = snapshot();
    source.environment.location.layer = 3;
    source.environment.static_world_objects = vec![StaticWorldObject {
        id: "placement.ordon-gate".into(),
        actor_type: "obj_gate".into(),
        placement_sha256: Digest([8; 32]),
        binding: ComponentBinding::Room {
            stage: "F_SP103".into(),
            room: 0,
        },
        parameters: BTreeMap::from([
            ("collision_active".into(), StateValue::Boolean(true)),
            ("phase".into(), StateValue::Text("placement".into())),
        ]),
    }];
    source.environment.persisted_object_controls = vec![PersistedObjectControl {
        object_id: "placement.ordon-gate".into(),
        fields: BTreeMap::from([
            ("opened".into(), StateValue::Boolean(true)),
            ("phase".into(), StateValue::Text("persisted".into())),
        ]),
    }];
    source.environment.live_world_objects = vec![LiveWorldObject {
        instance_id: "actor.ordon-gate.1".into(),
        static_object_id: Some("placement.ordon-gate".into()),
        actor_type: "obj_gate".into(),
        lifecycle: ActorLifecycle::Unloaded,
        fields: BTreeMap::from([("stale".into(), StateValue::Boolean(true))]),
    }];
    let operation = StateOperation::ReconstructActor {
        static_object_id: "placement.ordon-gate".into(),
        instance_id: "actor.ordon-gate.1".into(),
        required_layer: 3,
        initialization_fields: BTreeMap::from([
            ("collision_active".into(), StateValue::Boolean(false)),
            ("phase".into(), StateValue::Text("initialized".into())),
        ]),
    };
    let mut state = PlannerExecutionState::new(source).unwrap();
    state
        .apply_operations(
            "boundary.ordon-room-load",
            "snapshot.ordon-room-loaded",
            std::slice::from_ref(&operation),
        )
        .unwrap();
    let actor = &state.snapshot.environment.live_world_objects[0];
    assert_eq!(actor.lifecycle, ActorLifecycle::Loaded);
    assert_eq!(
        actor.fields,
        BTreeMap::from([
            ("collision_active".into(), StateValue::Boolean(false)),
            ("opened".into(), StateValue::Boolean(true)),
            ("phase".into(), StateValue::Text("initialized".into())),
        ])
    );

    let before = state.digest().unwrap();
    assert!(
        state
            .apply_operations(
                "boundary.duplicate-room-load",
                "snapshot.duplicate-room-load",
                &[operation],
            )
            .is_err()
    );
    assert_eq!(state.digest().unwrap(), before);
}
