
use super::*;
use crate::evaluation::{
    EvaluatedTruth, EvidencePolicy, FeasibilityMode, PredicateEvaluator, TransitionClassification,
};
use crate::execution::PlannerExecutionState;
use crate::identity::{
    CONTENT_IDENTITY_SCHEMA, ContentFingerprint, GamePlatform, GameRegion,
    RUNTIME_CONFIGURATION_SCHEMA,
};
use crate::logic::{FACT_CATALOG_SCHEMA, FactCatalog};
use crate::snapshot::{STATE_SNAPSHOT_SCHEMA, StateDiff, StateSnapshot};
use crate::state::{
    ActorLifecycle, BackingAttachment, CaptureStatus, ComponentBinding, ComponentBindingReference,
    ComponentKind, ComponentPayload, ComponentProvenance, EXECUTION_ENVIRONMENT_SCHEMA,
    ExecutionEnvironment, LiveWorldObject, PhysicalSlotObservation, PlayerForm, PlayerState,
    ProvenanceSourceKind, RuntimeFile, RuntimeFileLifecycle, RuntimeFileOrigin, SemanticLifetime,
    SerializationOwner, StateComponent,
};
use std::collections::{BTreeMap, BTreeSet};

fn context() -> (ContentIdentity, RuntimeConfiguration) {
    let content = ContentIdentity {
        schema: CONTENT_IDENTITY_SCHEMA.into(),
        id: "gcn-us-1.0-gz2e01".into(),
        fingerprint: ContentFingerprint {
            platform: GamePlatform::GameCube,
            region: GameRegion::Usa,
            revision: "1.0".into(),
            product_id: "GZ2E01".into(),
            executable_sha256: parse_digest(
                "e7f197436815e66c4a11df3d7bd557d66083b641ff8a8e76439f3caba7ae60e8",
            ),
            game_data_sha256: parse_digest(
                "0bc3bb229279d4b8a8c7cbe962b0bffdfecd35ff21e2d6761ad42e90a070f772",
            ),
            resource_manifest_sha256: parse_digest(
                "2ab36f6c1d9d551c1397e1cf59e13288d2684c973cb7bd0ad6878f5a3b3a2ab1",
            ),
        },
    };
    let runtime = RuntimeConfiguration {
        schema: RUNTIME_CONFIGURATION_SCHEMA.into(),
        content_sha256: GZ2E01_CONTENT_SHA256,
        language: "en".into(),
        settings: BTreeMap::new(),
    };
    (content, runtime)
}

fn component(
    id: &str,
    kind: ComponentKind,
    fields: impl IntoIterator<Item = (&'static str, StateValue)>,
) -> StateComponent {
    StateComponent {
        id: id.into(),
        component_kind: kind,
        payload: ComponentPayload::Structured {
            fields: fields
                .into_iter()
                .map(|(field, value)| (field.into(), value))
                .collect(),
        },
        binding: ComponentBinding::RuntimeFile {
            runtime_file_id: "file-0".into(),
        },
        lifetime: SemanticLifetime::RuntimeFile,
        serialization_owner: SerializationOwner::RuntimeFile {
            runtime_file_id: "file-0".into(),
        },
        provenance: vec![ComponentProvenance {
            source_kind: ProvenanceSourceKind::TraceObservation,
            source_id: "trace.reset-control".into(),
            source_sha256: Some(Digest([9; 32])),
            transition_id: None,
        }],
    }
}

fn raw_component(id: &str, kind: ComponentKind, byte_count: usize) -> StateComponent {
    let mut component = component(id, kind, []);
    component.payload = ComponentPayload::Raw {
        bytes: vec![0xaa; byte_count],
        known_mask: vec![0xff; byte_count],
    };
    component
}

fn loaded_stage_component() -> StateComponent {
    let mut component = raw_component(
        LOADED_STAGE_MEMORY_COMPONENT,
        ComponentKind::DungeonMemory,
        0x20,
    );
    component.binding = ComponentBinding::Stage {
        stage: "R_SP107".into(),
    };
    component.lifetime = SemanticLifetime::StageLoad;
    component.serialization_owner = SerializationOwner::StageBank {
        runtime_file_id: "file-0".into(),
        stage: "R_SP107".into(),
    };
    component
}

fn dungeon_session_label_component() -> StateComponent {
    let mut component = raw_component(
        DUNGEON_SESSION_LABEL_COMPONENT,
        ComponentKind::Custom {
            id: "observed-dungeon-session-switch-labels".into(),
        },
        4,
    );
    component.binding = ComponentBinding::Stage {
        stage: "R_SP107".into(),
    };
    component.lifetime = SemanticLifetime::StageLoad;
    component.serialization_owner = SerializationOwner::None;
    component
}

fn room_switch_label_component() -> StateComponent {
    let mut component = raw_component(
        ROOM_SWITCH_LABEL_COMPONENT,
        ComponentKind::Custom {
            id: "observed-room-switch-labels".into(),
        },
        4,
    );
    component.binding = ComponentBinding::Room {
        stage: "R_SP107".into(),
        room: 3,
    };
    component.lifetime = SemanticLifetime::RoomLoad;
    component
}

fn saved_dungeon_six_component() -> StateComponent {
    let mut component = component(
        DUNGEON_SIX_SAVE_COMPONENT,
        ComponentKind::DungeonMemory,
        [("key_count", StateValue::Unsigned(7))],
    );
    component.binding = ComponentBinding::Custom {
        kind_id: "saved-dungeon-memory".into(),
        context_id: "index-6".into(),
    };
    component
}

fn inventory_component() -> StateComponent {
    let mut component = component(INVENTORY_COMPONENT, ComponentKind::Inventory, []);
    component.payload = base_inventory_payload();
    let ComponentPayload::Structured { fields } = &mut component.payload else {
        unreachable!()
    };
    fields.insert("life".into(), StateValue::Unsigned(80));
    component
}

fn player_info_component() -> StateComponent {
    component(
        PLAYER_INFO_COMPONENT,
        ComponentKind::Custom {
            id: "player-info".into(),
        },
        [
            (
                "horse_name_bytes",
                StateValue::Bytes(DEFAULT_HORSE_NAME_BYTES.to_vec()),
            ),
            (
                "player_name_bytes",
                StateValue::Bytes(DEFAULT_PLAYER_NAME_BYTES.to_vec()),
            ),
            ("total_time_ticks", StateValue::Unsigned(0)),
            ("date_ipl_ticks", StateValue::Unsigned(0)),
        ],
    )
}

fn session_value_component(
    id: &str,
    fields: impl IntoIterator<Item = (&'static str, StateValue)>,
) -> StateComponent {
    let mut component = component(id, ComponentKind::Session, fields);
    component.binding = ComponentBinding::Session {
        session_id: "process".into(),
    };
    component.lifetime = SemanticLifetime::Session;
    component.serialization_owner = SerializationOwner::None;
    component
}

fn opening_process_control() -> StateComponent {
    let mut component = component(
        OPENING_PROCESS_CONTROL_COMPONENT,
        ComponentKind::Session,
        [("phase", StateValue::Text("phase_4".into()))],
    );
    component.binding = ComponentBinding::Session {
        session_id: "process".into(),
    };
    component.lifetime = SemanticLifetime::Session;
    component.serialization_owner = SerializationOwner::None;
    component
}

fn title_control() -> StateComponent {
    let mut component = component(
        TITLE_CONTROL_COMPONENT,
        ComponentKind::Title,
        [
            ("phase", StateValue::Text("key_wait".into())),
            ("reset_requested", StateValue::Boolean(false)),
            ("overlap_peek", StateValue::Boolean(false)),
            ("a_triggered", StateValue::Boolean(true)),
            ("start_triggered", StateValue::Boolean(false)),
        ],
    );
    component.binding = ComponentBinding::Session {
        session_id: "process".into(),
    };
    component.lifetime = SemanticLifetime::Session;
    component.serialization_owner = SerializationOwner::None;
    component
}

fn name_scene_control() -> StateComponent {
    let mut component = component(
        NAME_SCENE_CONTROL_COMPONENT,
        ComponentKind::Title,
        [("phase", StateValue::Text("create_file_select".into()))],
    );
    component.binding = ComponentBinding::Session {
        session_id: "process".into(),
    };
    component.lifetime = SemanticLifetime::Session;
    component.serialization_owner = SerializationOwner::None;
    component
}

fn save_menu_control(
    selected_index: u64,
    command_state: u64,
    use_type: u64,
    oil_gauge_backup: u64,
) -> StateComponent {
    let mut component = component(
        SAVE_MENU_CONTROL_COMPONENT,
        ComponentKind::Session,
        [
            ("buffer_loaded", StateValue::Boolean(true)),
            ("command_state_raw", StateValue::Unsigned(command_state)),
            ("oil_gauge_backup", StateValue::Unsigned(oil_gauge_backup)),
            ("phase", StateValue::Text("data_save_wait2".into())),
            ("selected_index_raw", StateValue::Unsigned(selected_index)),
            ("use_type_raw", StateValue::Unsigned(use_type)),
            ("wait_timer_raw", StateValue::Unsigned(0)),
        ],
    );
    component.binding = ComponentBinding::Session {
        session_id: "save-menu".into(),
    };
    component.lifetime = SemanticLifetime::Session;
    component.serialization_owner = SerializationOwner::None;
    component
}

fn reset_control_component() -> StateComponent {
    let mut component = component(
        RESET_CONTROL_COMPONENT,
        ComponentKind::Session,
        [
            ("reset_requested", StateValue::Boolean(true)),
            ("return_to_menu", StateValue::Boolean(false)),
            ("fader_status", StateValue::Unsigned(1)),
        ],
    );
    component.binding = ComponentBinding::Session {
        session_id: "process".into(),
    };
    component.lifetime = SemanticLifetime::Session;
    component.serialization_owner = SerializationOwner::None;
    component
}

fn retarget_runtime(snapshot: &mut StateSnapshot, runtime_file_id: &str) {
    let source_runtime_file_id = snapshot.environment.active_runtime_file.id.clone();
    for component in &mut snapshot.environment.components {
        if let ComponentBinding::RuntimeFile {
            runtime_file_id: bound_runtime,
        } = &mut component.binding
            && *bound_runtime == source_runtime_file_id
        {
            *bound_runtime = runtime_file_id.into();
        }
        match &mut component.serialization_owner {
            SerializationOwner::RuntimeFile {
                runtime_file_id: owner_runtime,
            }
            | SerializationOwner::StageBank {
                runtime_file_id: owner_runtime,
                ..
            } if *owner_runtime == source_runtime_file_id => {
                *owner_runtime = runtime_file_id.into();
            }
            _ => {}
        }
    }
    snapshot.environment.active_runtime_file = RuntimeFile {
        id: runtime_file_id.into(),
        origin: RuntimeFileOrigin::NewFile,
        backing: BackingAttachment::MemoryOnly,
        allowed_serialization_targets: vec![
            PhysicalSlotId(1),
            PhysicalSlotId(2),
            PhysicalSlotId(3),
        ],
        lifecycle: RuntimeFileLifecycle::Active,
    };
}

fn component_for<'a>(state: &'a PlannerExecutionState, id: &str) -> &'a StateComponent {
    state
        .snapshot
        .environment
        .components
        .iter()
        .find(|component| component.id == id)
        .unwrap()
}

fn fields_for<'a>(state: &'a PlannerExecutionState, id: &str) -> &'a BTreeMap<String, StateValue> {
    let ComponentPayload::Structured { fields } = &component_for(state, id).payload else {
        panic!("{id} should be structured")
    };
    fields
}

fn set_structured_field(
    state: &mut PlannerExecutionState,
    component_id: &str,
    field: &str,
    value: StateValue,
) {
    let component = state
        .snapshot
        .environment
        .components
        .iter_mut()
        .find(|component| component.id == component_id)
        .unwrap();
    let ComponentPayload::Structured { fields } = &mut component.payload else {
        panic!("{component_id} should be structured")
    };
    fields.insert(field.into(), value);
}

fn snapshot(runtime: RuntimeConfiguration) -> StateSnapshot {
    StateSnapshot {
        schema: STATE_SNAPSHOT_SCHEMA.into(),
        id: "snapshot.before-reset".into(),
        sequence: 0,
        environment: ExecutionEnvironment {
            schema: EXECUTION_ENVIRONMENT_SCHEMA.into(),
            runtime_configuration: runtime,
            active_runtime_file: RuntimeFile {
                id: "file-0".into(),
                origin: RuntimeFileOrigin::TitleFile0,
                backing: BackingAttachment::MemoryOnly,
                allowed_serialization_targets: vec![
                    PhysicalSlotId(1),
                    PhysicalSlotId(2),
                    PhysicalSlotId(3),
                ],
                lifecycle: RuntimeFileLifecycle::Active,
            },
            inactive_runtime_files: Vec::new(),
            physical_slots: Vec::new(),
            physical_slot_observations: Vec::new(),
            execution_context: ExecutionContext::World,
            location: SceneLocation {
                stage: "R_SP107".into(),
                room: 3,
                layer: 0,
                spawn: 0,
            },
            player: PlayerState {
                form: PlayerForm::Wolf,
                mount: None,
                position: [0.0; 3],
                attention_position: None,
                rotation: [0; 3],
                has_control: Some(true),
                action: "idle".into(),
            },
            components: vec![
                dungeon_session_label_component(),
                raw_component(
                    OBSERVED_EVENT_COMPONENT,
                    ComponentKind::Custom {
                        id: "observed-event-flag-labels".into(),
                    },
                    4,
                ),
                loaded_stage_component(),
                raw_component(
                    PERSISTENT_EVENT_COMPONENT,
                    ComponentKind::Custom {
                        id: "persistent-event-registers".into(),
                    },
                    256,
                ),
                room_switch_label_component(),
                raw_component(
                    OBSERVED_TEMPORARY_COMPONENT,
                    ComponentKind::Custom {
                        id: "observed-temporary-flag-labels".into(),
                    },
                    4,
                ),
                raw_component(
                    TEMPORARY_EVENT_COMPONENT,
                    ComponentKind::TemporaryFlags,
                    256,
                ),
                inventory_component(),
                reset_control_component(),
                component(
                    RESTART_COMPONENT,
                    ComponentKind::Restart,
                    [("room_param", StateValue::Unsigned(0xc9))],
                ),
                component(
                    RETURN_PLACE_COMPONENT,
                    ComponentKind::PersistentSave,
                    [
                        ("player_status", StateValue::Unsigned(9)),
                        ("room", StateValue::Signed(3)),
                        ("stage", StateValue::Text("R_SP107".into())),
                    ],
                ),
                component(
                    RUNTIME_FILE_HEADER_COMPONENT,
                    ComponentKind::Session,
                    [
                        ("data_num_raw", StateValue::Unsigned(3)),
                        ("new_file_raw", StateValue::Unsigned(9)),
                        ("no_file_raw", StateValue::Unsigned(7)),
                    ],
                ),
                saved_dungeon_six_component(),
                player_info_component(),
                raw_component(
                    LIGHT_DROP_COMPONENT,
                    ComponentKind::Custom {
                        id: "player-light-drop".into(),
                    },
                    5,
                ),
                session_value_component(
                    ACTIVE_VIBRATION_COMPONENT,
                    [("enabled_raw", StateValue::Unsigned(0))],
                ),
                session_value_component(
                    SAVE_STAGE_DISPLAY_COMPONENT,
                    [("stage", StateValue::Text("stale".into()))],
                ),
            ],
            static_world_objects: Vec::new(),
            spatial_volumes: Vec::new(),
            spatial_connections: Vec::new(),
            spatial_planes: Vec::new(),
            persisted_object_controls: Vec::new(),
            live_world_objects: vec![LiveWorldObject {
                instance_id: "actor.retained-world-probe".into(),
                static_object_id: None,
                actor_type: "probe".into(),
                lifecycle: ActorLifecycle::Loaded,
                fields: BTreeMap::from([("active".into(), StateValue::Boolean(true))]),
            }],
        },
        semantic_observations: Vec::new(),
    }
}

#[test]
fn reset_prefix_enters_process_without_claiming_pending_map_is_loaded() {
    let (content, runtime) = context();
    let catalog = gz2e01_reset_to_opening_mechanics(&content, &runtime).unwrap();
    let transition = catalog
        .transitions
        .iter()
        .find(|transition| transition.id == "transition.gz2e01.reset-to-opening")
        .unwrap();
    let facts = FactCatalog {
        schema: FACT_CATALOG_SCHEMA.into(),
        aliases: Vec::new(),
        derived_facts: Vec::new(),
    };
    let before = snapshot(runtime);
    let evaluator = PredicateEvaluator::new(
        &before,
        &facts,
        &[],
        &BTreeMap::new(),
        EvidencePolicy::RESEARCH,
    )
    .unwrap();
    let assessment = evaluator.assess_transition(
        transition,
        &BTreeSet::new(),
        &BTreeSet::new(),
        FeasibilityMode::Modeled,
    );
    assert_eq!(
        assessment.classification,
        TransitionClassification::Executable
    );

    let mut state = PlannerExecutionState::new(before.clone()).unwrap();
    state
        .apply_operations(
            &transition.id,
            "snapshot.after-reset-prefix",
            &transition.activation.effects,
        )
        .unwrap();
    assert_eq!(
        state.snapshot.environment.execution_context,
        ExecutionContext::Process {
            process_name: "PROC_OPENING_SCENE".into(),
            pending_world_load: Some(SceneLocation {
                stage: "F_SP102".into(),
                room: 0,
                layer: 10,
                spawn: 100,
            }),
        }
    );
    assert_eq!(state.snapshot.environment.location.stage, "R_SP107");
    assert_eq!(
        ComponentBindingReference::CurrentStage.resolve(&state.snapshot.environment),
        None
    );
    let diff = StateDiff::between(
        &before,
        &state.snapshot,
        crate::state::BoundaryKind::TitleReturn,
    )
    .unwrap();
    assert!(diff.execution_context_changed);
    assert_eq!(diff.execution_context_before, ExecutionContext::World);
    assert_eq!(
        diff.execution_context_after,
        state.snapshot.environment.execution_context
    );
    assert!(!diff.location_changed);

    let evaluator = PredicateEvaluator::new(
        &state.snapshot,
        &facts,
        &[],
        &BTreeMap::new(),
        EvidencePolicy::RESEARCH,
    )
    .unwrap();
    assert_eq!(
        evaluator.evaluate(&PredicateExpression::Compare {
            left: ValueReference::LocationStage,
            operator: ComparisonOperator::Equal,
            right: ValueReference::Literal {
                value: StateValue::Text("R_SP107".into()),
            },
        }),
        EvaluatedTruth::Unknown
    );
    assert_eq!(
        evaluator.resolve_value(&ValueReference::ExecutionProcess),
        Some(StateValue::Text("PROC_OPENING_SCENE".into()))
    );
    assert_eq!(
        evaluator.resolve_value(&ValueReference::ActorField {
            instance_id: "actor.retained-world-probe".into(),
            field: "active".into(),
        }),
        None
    );
    let opening = catalog
        .transitions
        .iter()
        .find(|transition| transition.id == "transition.gz2e01.opening-file0-initialize")
        .unwrap();
    let evaluator = PredicateEvaluator::new(
        &state.snapshot,
        &facts,
        &[],
        &BTreeMap::new(),
        EvidencePolicy::RESEARCH,
    )
    .unwrap();
    assert_eq!(
        evaluator
            .assess_transition(
                opening,
                &BTreeSet::new(),
                &BTreeSet::new(),
                FeasibilityMode::Modeled,
            )
            .classification,
        TransitionClassification::FeasibilityUnknown,
        "the pending load alone must not prove that opening phases 0-3 reached phase 4"
    );
    state
        .snapshot
        .environment
        .components
        .push(opening_process_control());
    state
        .snapshot
        .environment
        .components
        .sort_by(|left, right| left.id.cmp(&right.id));
    let stage_owner = SerializationOwner::StageBank {
        runtime_file_id: "file-0".into(),
        stage: "R_SP107".into(),
    };
    state
        .serialized_components
        .insert(stage_owner.clone(), vec![loaded_stage_component()]);
    state.validate().unwrap();
    let restart_before = fields_for(&state, RESTART_COMPONENT).clone();
    let header_before = fields_for(&state, "runtime-file.header").clone();

    let evaluator = PredicateEvaluator::new(
        &state.snapshot,
        &facts,
        &[],
        &BTreeMap::new(),
        EvidencePolicy::RESEARCH,
    )
    .unwrap();
    assert_eq!(
        evaluator
            .assess_transition(
                opening,
                &BTreeSet::new(),
                &BTreeSet::new(),
                FeasibilityMode::Modeled,
            )
            .classification,
        TransitionClassification::Executable
    );
    state
        .apply_operations(
            &opening.id,
            "snapshot.after-opening-file0-init",
            &opening.activation.effects,
        )
        .unwrap();
    assert_eq!(state.snapshot.environment.location.stage, "F_SP102");
    assert_eq!(
        state.snapshot.environment.execution_context,
        ExecutionContext::Process {
            process_name: "PROC_OPENING_SCENE".into(),
            pending_world_load: None,
        }
    );
    assert_eq!(
        fields_for(&state, INVENTORY_COMPONENT).get("equipment"),
        Some(&StateValue::Bytes(vec![0x2f, 0x28, 0x2c, 0xff, 0xff, 0]))
    );
    assert_eq!(
        fields_for(&state, INVENTORY_COMPONENT).get("inventory"),
        Some(&StateValue::Bytes(vec![0xff; 24]))
    );
    assert_eq!(
        fields_for(&state, INVENTORY_COMPONENT).get("acquired_item_bits"),
        Some(&StateValue::Bytes(vec![0; 32]))
    );
    assert_eq!(
        fields_for(&state, INVENTORY_COMPONENT).get("collect_item_bits"),
        Some(&StateValue::Bytes(vec![0, 1, 4, 0, 0, 0, 0, 0]))
    );
    assert_eq!(
        fields_for(&state, RETURN_PLACE_COMPONENT).get("stage"),
        Some(&StateValue::Text("F_SP108".into()))
    );
    assert_eq!(fields_for(&state, RESTART_COMPONENT), &restart_before);
    assert_eq!(fields_for(&state, "runtime-file.header"), &header_before);
    let event = component_for(&state, PERSISTENT_EVENT_COMPONENT);
    let ComponentPayload::Raw { bytes, known_mask } = &event.payload else {
        panic!("persistent event registers should be exact raw bytes")
    };
    assert_eq!(bytes.len(), 256);
    assert_eq!(bytes[6], 1);
    assert!(
        bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| index == 6 || *byte == 0)
    );
    assert_eq!(known_mask, &vec![0xff; 256]);
    let ComponentPayload::Raw {
        bytes: stage_bytes,
        known_mask: stage_known_mask,
    } = &component_for(&state, LOADED_STAGE_MEMORY_COMPONENT).payload
    else {
        panic!("loaded stage memory should be exact raw bytes")
    };
    assert_eq!(stage_bytes, &vec![0; 0x20]);
    assert_eq!(&stage_known_mask[..0x1e], &vec![0xff; 0x1e]);
    assert_eq!(&stage_known_mask[0x1e..], &[0, 0]);
    for component_id in [
        OBSERVED_EVENT_COMPONENT,
        OBSERVED_TEMPORARY_COMPONENT,
        DUNGEON_SESSION_LABEL_COMPONENT,
        ROOM_SWITCH_LABEL_COMPONENT,
    ] {
        assert_eq!(
            component_for(&state, component_id).payload,
            ComponentPayload::Unknown {
                expected_bytes: None
            }
        );
    }
    assert_eq!(
        state.serialized_components[&stage_owner][0].payload,
        ComponentPayload::Unknown {
            expected_bytes: Some(0x20)
        }
    );
    assert_eq!(
        fields_for(&state, OPENING_PROCESS_CONTROL_COMPONENT).get("phase"),
        Some(&StateValue::Text("complete".into()))
    );
    let evaluator = PredicateEvaluator::new(
        &state.snapshot,
        &facts,
        &[],
        &BTreeMap::new(),
        EvidencePolicy::RESEARCH,
    )
    .unwrap();
    assert_eq!(
        evaluator
            .assess_transition(
                opening,
                &BTreeSet::new(),
                &BTreeSet::new(),
                FeasibilityMode::Modeled,
            )
            .classification,
        TransitionClassification::GuardBlocked
    );
}

#[test]
fn new_runtime_enters_a_fresh_title_file_zero_lifetime_atomically() {
    let (content, runtime) = context();
    let catalog = gz2e01_reset_to_opening_mechanics(&content, &runtime).unwrap();
    let facts = FactCatalog {
        schema: FACT_CATALOG_SCHEMA.into(),
        aliases: Vec::new(),
        derived_facts: Vec::new(),
    };
    let mut before = snapshot(runtime);
    retarget_runtime(&mut before, "new-file");
    let mut state = PlannerExecutionState::new(before).unwrap();
    let reset = catalog
        .transitions
        .iter()
        .find(|transition| transition.id == "transition.gz2e01.reset-to-opening")
        .unwrap();
    state
        .apply_operations(
            &reset.id,
            "snapshot.new-file-opening-requested",
            &reset.activation.effects,
        )
        .unwrap();
    state
        .snapshot
        .environment
        .components
        .push(opening_process_control());
    state
        .snapshot
        .environment
        .components
        .sort_by(|left, right| left.id.cmp(&right.id));
    state.validate().unwrap();

    let existing_file_zero = catalog
        .transitions
        .iter()
        .find(|transition| transition.id == "transition.gz2e01.opening-file0-initialize")
        .unwrap();
    let enter_file_zero = catalog
        .transitions
        .iter()
        .find(|transition| transition.id == "transition.gz2e01.opening-enter-and-initialize-file0")
        .unwrap();
    let evaluator = PredicateEvaluator::new(
        &state.snapshot,
        &facts,
        &[],
        &BTreeMap::new(),
        EvidencePolicy::RESEARCH,
    )
    .unwrap();
    assert_eq!(
        evaluator
            .assess_transition(
                existing_file_zero,
                &BTreeSet::new(),
                &BTreeSet::new(),
                FeasibilityMode::Modeled,
            )
            .classification,
        TransitionClassification::GuardBlocked
    );
    assert_eq!(
        evaluator
            .assess_transition(
                enter_file_zero,
                &BTreeSet::new(),
                &BTreeSet::new(),
                FeasibilityMode::Modeled,
            )
            .classification,
        TransitionClassification::Executable
    );

    let header_before = fields_for(&state, "runtime-file.header").clone();
    state
        .apply_operations(
            &enter_file_zero.id,
            "snapshot.title-file-zero-initialized",
            &enter_file_zero.activation.effects,
        )
        .unwrap();

    let active = &state.snapshot.environment.active_runtime_file;
    assert_eq!(active.id, "new-file.title-file-0");
    assert_eq!(active.origin, RuntimeFileOrigin::TitleFile0);
    assert_eq!(active.backing, BackingAttachment::MemoryOnly);
    assert_eq!(
        active.allowed_serialization_targets,
        vec![PhysicalSlotId(1), PhysicalSlotId(2), PhysicalSlotId(3)]
    );
    assert_eq!(
        state
            .snapshot
            .environment
            .inactive_runtime_files
            .iter()
            .find(|runtime| runtime.id == "new-file")
            .unwrap()
            .lifecycle,
        RuntimeFileLifecycle::Ended
    );
    assert_eq!(fields_for(&state, "runtime-file.header"), &header_before);
    assert_eq!(
        fields_for(&state, INVENTORY_COMPONENT).get("equipment"),
        Some(&StateValue::Bytes(vec![0x2f, 0x28, 0x2c, 0xff, 0xff, 0]))
    );
    assert!(
        state
            .snapshot
            .environment
            .components
            .iter()
            .filter(|component| {
                matches!(
                    component.serialization_owner,
                    SerializationOwner::RuntimeFile { .. } | SerializationOwner::StageBank { .. }
                )
            })
            .all(|component| match &component.serialization_owner {
                SerializationOwner::RuntimeFile { runtime_file_id }
                | SerializationOwner::StageBank {
                    runtime_file_id, ..
                } => runtime_file_id == "new-file.title-file-0",
                _ => unreachable!(),
            })
    );
}

#[test]
fn title_input_and_file_select_create_reset_only_the_audited_file_state() {
    let (content, runtime) = context();
    let catalog = gz2e01_reset_to_opening_mechanics(&content, &runtime).unwrap();
    let facts = FactCatalog {
        schema: FACT_CATALOG_SCHEMA.into(),
        aliases: Vec::new(),
        derived_facts: Vec::new(),
    };
    let transition = |id: &str| {
        catalog
            .transitions
            .iter()
            .find(|transition| transition.id == id)
            .unwrap()
    };
    let mut state = PlannerExecutionState::new(snapshot(runtime)).unwrap();
    state
        .apply_operations(
            "transition.gz2e01.reset-to-opening",
            "snapshot.title-chain.reset",
            &transition("transition.gz2e01.reset-to-opening")
                .activation
                .effects,
        )
        .unwrap();
    state
        .snapshot
        .environment
        .components
        .push(opening_process_control());
    state
        .snapshot
        .environment
        .components
        .sort_by(|left, right| left.id.cmp(&right.id));
    state
        .apply_operations(
            "transition.gz2e01.opening-file0-initialize",
            "snapshot.title-chain.opening-complete",
            &transition("transition.gz2e01.opening-file0-initialize")
                .activation
                .effects,
        )
        .unwrap();
    assert_eq!(
        fields_for(&state, INVENTORY_COMPONENT)["equipment"],
        StateValue::Bytes(vec![0x2f, 0x28, 0x2c, 0xff, 0xff, 0])
    );
    let ComponentPayload::Raw { bytes, .. } =
        &component_for(&state, PERSISTENT_EVENT_COMPONENT).payload
    else {
        unreachable!()
    };
    assert_eq!(bytes[6] & 1, 1);

    state.snapshot.environment.components.push(title_control());
    state
        .snapshot
        .environment
        .components
        .sort_by(|left, right| left.id.cmp(&right.id));
    let evaluator = PredicateEvaluator::new(
        &state.snapshot,
        &facts,
        &[],
        &BTreeMap::new(),
        EvidencePolicy::RESEARCH,
    )
    .unwrap();
    assert_eq!(
        evaluator
            .assess_transition(
                transition("transition.gz2e01.title-key-accept"),
                &BTreeSet::new(),
                &BTreeSet::new(),
                FeasibilityMode::Modeled,
            )
            .classification,
        TransitionClassification::Executable
    );
    state
        .apply_operations(
            "transition.gz2e01.title-key-accept",
            "snapshot.title-chain.key-accepted",
            &transition("transition.gz2e01.title-key-accept")
                .activation
                .effects,
        )
        .unwrap();
    state
        .apply_operations(
            "transition.gz2e01.title-request-name-scene",
            "snapshot.title-chain.name-requested",
            &transition("transition.gz2e01.title-request-name-scene")
                .activation
                .effects,
        )
        .unwrap();
    assert_eq!(
        state.snapshot.environment.execution_context,
        ExecutionContext::Process {
            process_name: "PROC_OPENING_SCENE".into(),
            pending_world_load: None,
        },
        "a process ChangeReq must not masquerade as completed activation"
    );

    state
        .snapshot
        .environment
        .components
        .push(name_scene_control());
    state
        .snapshot
        .environment
        .components
        .sort_by(|left, right| left.id.cmp(&right.id));
    let evaluator = PredicateEvaluator::new(
        &state.snapshot,
        &facts,
        &[],
        &BTreeMap::new(),
        EvidencePolicy::RESEARCH,
    )
    .unwrap();
    assert_eq!(
        evaluator
            .assess_transition(
                transition("transition.gz2e01.name-scene-file-select-initialize"),
                &BTreeSet::new(),
                &BTreeSet::new(),
                FeasibilityMode::Modeled,
            )
            .classification,
        TransitionClassification::GuardBlocked,
        "a create-phase observation cannot activate while opening is still the current process"
    );
    // Independent scheduler/process observation: NAME_SCENE is now the
    // active process and has reached the file-select construction phase.
    state.snapshot.environment.execution_context = ExecutionContext::Process {
        process_name: "PROC_NAME_SCENE".into(),
        pending_world_load: None,
    };
    state.validate().unwrap();
    let evaluator = PredicateEvaluator::new(
        &state.snapshot,
        &facts,
        &[],
        &BTreeMap::new(),
        EvidencePolicy::RESEARCH,
    )
    .unwrap();
    assert_eq!(
        evaluator
            .assess_transition(
                transition("transition.gz2e01.name-scene-file-select-initialize"),
                &BTreeSet::new(),
                &BTreeSet::new(),
                FeasibilityMode::Modeled,
            )
            .classification,
        TransitionClassification::Executable
    );
    let active_runtime_before = state.snapshot.environment.active_runtime_file.clone();
    let inactive_runtimes_before = state.snapshot.environment.inactive_runtime_files.clone();
    let restart_before = fields_for(&state, RESTART_COMPONENT).clone();
    state
        .apply_operations(
            "transition.gz2e01.name-scene-file-select-initialize",
            "snapshot.title-chain.file-select-open",
            &transition("transition.gz2e01.name-scene-file-select-initialize")
                .activation
                .effects,
        )
        .unwrap();

    assert_eq!(
        state.snapshot.environment.active_runtime_file,
        active_runtime_before
    );
    assert_eq!(
        state.snapshot.environment.inactive_runtime_files,
        inactive_runtimes_before
    );
    assert_eq!(fields_for(&state, RESTART_COMPONENT), &restart_before);
    assert_eq!(
        fields_for(&state, RUNTIME_FILE_HEADER_COMPONENT),
        &BTreeMap::from([
            ("data_num_raw".into(), StateValue::Unsigned(3)),
            ("new_file_raw".into(), StateValue::Unsigned(0)),
            ("no_file_raw".into(), StateValue::Unsigned(0)),
        ])
    );
    assert_eq!(
        fields_for(&state, INVENTORY_COMPONENT)["equipment"],
        StateValue::Bytes(vec![0x2e, 0xff, 0xff, 0xff, 0xff, 0])
    );
    assert_eq!(
        fields_for(&state, INVENTORY_COMPONENT)["collect_item_bits"],
        StateValue::Bytes(vec![0; 8])
    );
    let ComponentPayload::Raw { bytes, known_mask } =
        &component_for(&state, PERSISTENT_EVENT_COMPONENT).payload
    else {
        unreachable!()
    };
    assert_eq!(bytes, &vec![0; 256]);
    assert_eq!(known_mask, &vec![0xff; 256]);
    assert_eq!(
        fields_for(&state, NAME_SCENE_CONTROL_COMPONENT)["phase"],
        StateValue::Text("file_select_open".into())
    );
    assert_eq!(
        fields_for(&state, TITLE_CONTROL_COMPONENT)["phase"],
        StateValue::Text("scene_requested".into())
    );
    let evaluator = PredicateEvaluator::new(
        &state.snapshot,
        &facts,
        &[],
        &BTreeMap::new(),
        EvidencePolicy::RESEARCH,
    )
    .unwrap();
    assert_eq!(
        evaluator
            .assess_transition(
                transition("transition.gz2e01.name-scene-file-select-initialize"),
                &BTreeSet::new(),
                &BTreeSet::new(),
                FeasibilityMode::Modeled,
            )
            .classification,
        TransitionClassification::GuardBlocked
    );
}

#[test]
fn file_select_branches_are_exclusive_and_keep_buffer_card_and_runtime_state_distinct() {
    let (content, runtime) = context();
    let catalog = gz2e01_reset_to_opening_mechanics(&content, &runtime).unwrap();
    let facts = FactCatalog {
        schema: FACT_CATALOG_SCHEMA.into(),
        aliases: Vec::new(),
        derived_facts: Vec::new(),
    };
    let transition = |id: &str| {
        catalog
            .transitions
            .iter()
            .find(|transition| transition.id == id)
            .unwrap()
    };
    let make_file_select_state = |with_existing_slot: bool| {
        let mut before = snapshot(runtime.clone());
        before.environment.execution_context = ExecutionContext::Process {
            process_name: "PROC_NAME_SCENE".into(),
            pending_world_load: None,
        };
        before.environment.components.push(name_scene_control());
        before
            .environment
            .components
            .sort_by(|left, right| left.id.cmp(&right.id));
        if with_existing_slot {
            let inventory = before
                .environment
                .components
                .iter_mut()
                .find(|component| component.id == INVENTORY_COMPONENT)
                .unwrap();
            let ComponentPayload::Structured { fields } = &mut inventory.payload else {
                unreachable!()
            };
            fields.insert("life".into(), StateValue::Unsigned(4));
            let mut items = vec![ITEM_NONE; 24];
            items[9] = ITEM_DOUBLE_CLAWSHOT;
            fields.insert("inventory".into(), StateValue::Bytes(items));
            fields.insert("item_lineup".into(), StateValue::Bytes(vec![23; 24]));
            fields.insert("vibration".into(), StateValue::Unsigned(1));
            let player_info = before
                .environment
                .components
                .iter_mut()
                .find(|component| component.id == PLAYER_INFO_COMPONENT)
                .unwrap();
            let ComponentPayload::Structured { fields } = &mut player_info.payload else {
                unreachable!()
            };
            fields.insert(
                "player_name_bytes".into(),
                StateValue::Bytes(b"SlotOne\0".to_vec()),
            );
            let dungeon_six = before
                .environment
                .components
                .iter_mut()
                .find(|component| component.id == DUNGEON_SIX_SAVE_COMPONENT)
                .unwrap();
            let ComponentPayload::Structured { fields } = &mut dungeon_six.payload else {
                unreachable!()
            };
            fields.insert("key_count".into(), StateValue::Unsigned(5));
        }
        let mut state = PlannerExecutionState::new(before).unwrap();
        if with_existing_slot {
            state
                .apply_operations(
                    "boundary.seed-existing-slot-1",
                    "snapshot.slot-1-seeded",
                    &[StateOperation::SaveRuntimeToSlot {
                        source_runtime_file_id: "file-0".into(),
                        destination_slot: PhysicalSlotId(1),
                        destination_persistent_file_id: "existing-slot-1-image".into(),
                        runtime_component_ids: vec![
                            PERSISTENT_EVENT_COMPONENT.into(),
                            INVENTORY_COMPONENT.into(),
                            RETURN_PLACE_COMPONENT.into(),
                            DUNGEON_SIX_SAVE_COMPONENT.into(),
                            PLAYER_INFO_COMPONENT.into(),
                            LIGHT_DROP_COMPONENT.into(),
                        ],
                        stage_bank_stages: Vec::new(),
                    }],
                )
                .unwrap();
        }
        state
            .apply_operations(
                "transition.gz2e01.name-scene-file-select-initialize",
                "snapshot.file-select-open",
                &transition("transition.gz2e01.name-scene-file-select-initialize")
                    .activation
                    .effects,
            )
            .unwrap();
        state
    };
    let classify = |state: &PlannerExecutionState, id: &str| {
        PredicateEvaluator::new(
            &state.snapshot,
            &facts,
            &[],
            &BTreeMap::new(),
            EvidencePolicy::RESEARCH,
        )
        .unwrap()
        .assess_transition(
            transition(id),
            &BTreeSet::new(),
            &BTreeSet::new(),
            FeasibilityMode::Modeled,
        )
        .classification
    };

    let mut blank = make_file_select_state(false);
    set_structured_field(
        &mut blank,
        NAME_SCENE_CONTROL_COMPONENT,
        "selected_entry_kind",
        StateValue::Text("new".into()),
    );
    set_structured_field(
        &mut blank,
        NAME_SCENE_CONTROL_COMPONENT,
        "selected_index_raw",
        StateValue::Unsigned(1),
    );
    blank.validate().unwrap();
    assert_eq!(
        classify(&blank, "transition.gz2e01.file-select-blank-slot-2"),
        TransitionClassification::Executable
    );
    assert_eq!(
        classify(&blank, "transition.gz2e01.file-select-blank-slot-1"),
        TransitionClassification::GuardBlocked
    );
    assert_eq!(
        classify(&blank, "transition.gz2e01.file-select-open-existing-slot"),
        TransitionClassification::GuardBlocked
    );
    assert_eq!(
        classify(&blank, "transition.gz2e01.file-select-proceed-without-card"),
        TransitionClassification::GuardBlocked
    );
    let blank_runtime = blank.snapshot.environment.active_runtime_file.clone();
    blank
        .apply_operations(
            "transition.gz2e01.file-select-blank-slot-2",
            "snapshot.blank-slot-2-selected",
            &transition("transition.gz2e01.file-select-blank-slot-2")
                .activation
                .effects,
        )
        .unwrap();
    assert_eq!(
        blank.snapshot.environment.active_runtime_file, blank_runtime,
        "blank selection does not load or end the live title-origin runtime"
    );
    assert!(blank.snapshot.environment.physical_slots.is_empty());
    assert_eq!(
        fields_for(&blank, RUNTIME_FILE_HEADER_COMPONENT)["new_file_raw"],
        StateValue::Unsigned(128)
    );
    assert_eq!(
        fields_for(&blank, RUNTIME_FILE_HEADER_COMPONENT)["data_num_raw"],
        StateValue::Unsigned(1)
    );
    assert_eq!(
        fields_for(&blank, PLAYER_INFO_COMPONENT)["player_name_bytes"],
        StateValue::Bytes(DEFAULT_PLAYER_NAME_BYTES.to_vec())
    );
    let player_name = StateValue::Bytes(b"Midna\0".to_vec());
    set_structured_field(
        &mut blank,
        NAME_SCENE_CONTROL_COMPONENT,
        "submitted_name_bytes",
        player_name.clone(),
    );
    set_structured_field(
        &mut blank,
        NAME_SCENE_CONTROL_COMPONENT,
        "input_result_raw",
        StateValue::Unsigned(2),
    );
    assert_eq!(
        classify(&blank, "transition.gz2e01.file-select-player-name-confirm"),
        TransitionClassification::Executable
    );
    assert_eq!(
        classify(
            &blank,
            "transition.gz2e01.file-select-player-name-cancel-to-data-select"
        ),
        TransitionClassification::GuardBlocked
    );
    blank
        .apply_operations(
            "transition.gz2e01.file-select-player-name-confirm",
            "snapshot.player-name-confirmed",
            &transition("transition.gz2e01.file-select-player-name-confirm")
                .activation
                .effects,
        )
        .unwrap();
    assert_eq!(
        fields_for(&blank, PLAYER_INFO_COMPONENT)["player_name_bytes"],
        player_name
    );
    set_structured_field(
        &mut blank,
        NAME_SCENE_CONTROL_COMPONENT,
        "fade_timer_raw",
        StateValue::Unsigned(0),
    );
    blank
        .apply_operations(
            "transition.gz2e01.file-select-player-name-fade-complete",
            "snapshot.horse-name-initialized",
            &transition("transition.gz2e01.file-select-player-name-fade-complete")
                .activation
                .effects,
        )
        .unwrap();
    assert_eq!(
        fields_for(&blank, PLAYER_INFO_COMPONENT)["horse_name_bytes"],
        StateValue::Bytes(DEFAULT_HORSE_NAME_BYTES.to_vec())
    );
    set_structured_field(
        &mut blank,
        NAME_SCENE_CONTROL_COMPONENT,
        "fade_timer_raw",
        StateValue::Unsigned(0),
    );
    set_structured_field(
        &mut blank,
        NAME_SCENE_CONTROL_COMPONENT,
        "reset_requested",
        StateValue::Boolean(false),
    );
    blank
        .apply_operations(
            "transition.gz2e01.file-select-horse-name-entry-ready",
            "snapshot.horse-name-ready",
            &transition("transition.gz2e01.file-select-horse-name-entry-ready")
                .activation
                .effects,
        )
        .unwrap();

    // Exercise the exact horse-name Back chain before confirming. It must
    // return to player-name input without undoing the confirmed player name.
    set_structured_field(
        &mut blank,
        NAME_SCENE_CONTROL_COMPONENT,
        "input_result_raw",
        StateValue::Unsigned(1),
    );
    blank
        .apply_operations(
            "transition.gz2e01.file-select-horse-name-back",
            "snapshot.horse-name-backed-out",
            &transition("transition.gz2e01.file-select-horse-name-back")
                .activation
                .effects,
        )
        .unwrap();
    set_structured_field(
        &mut blank,
        NAME_SCENE_CONTROL_COMPONENT,
        "fade_timer_raw",
        StateValue::Unsigned(0),
    );
    blank
        .apply_operations(
            "transition.gz2e01.file-select-player-name-back-fade-complete",
            "snapshot.player-name-back-moving",
            &transition("transition.gz2e01.file-select-player-name-back-fade-complete")
                .activation
                .effects,
        )
        .unwrap();
    set_structured_field(
        &mut blank,
        NAME_SCENE_CONTROL_COMPONENT,
        "fade_timer_raw",
        StateValue::Unsigned(0),
    );
    blank
        .apply_operations(
            "transition.gz2e01.file-select-player-name-back-ready",
            "snapshot.player-name-ready-again",
            &transition("transition.gz2e01.file-select-player-name-back-ready")
                .activation
                .effects,
        )
        .unwrap();
    assert_eq!(
        fields_for(&blank, PLAYER_INFO_COMPONENT)["player_name_bytes"],
        player_name
    );

    // Reconfirm the player name, finish the two fades, and confirm the
    // horse name. This is the real path to selection_end.
    set_structured_field(
        &mut blank,
        NAME_SCENE_CONTROL_COMPONENT,
        "input_result_raw",
        StateValue::Unsigned(2),
    );
    blank
        .apply_operations(
            "transition.gz2e01.file-select-player-name-confirm",
            "snapshot.player-name-reconfirmed",
            &transition("transition.gz2e01.file-select-player-name-confirm")
                .activation
                .effects,
        )
        .unwrap();
    set_structured_field(
        &mut blank,
        NAME_SCENE_CONTROL_COMPONENT,
        "fade_timer_raw",
        StateValue::Unsigned(0),
    );
    blank
        .apply_operations(
            "transition.gz2e01.file-select-player-name-fade-complete",
            "snapshot.horse-name-reinitialized",
            &transition("transition.gz2e01.file-select-player-name-fade-complete")
                .activation
                .effects,
        )
        .unwrap();
    set_structured_field(
        &mut blank,
        NAME_SCENE_CONTROL_COMPONENT,
        "fade_timer_raw",
        StateValue::Unsigned(0),
    );
    blank
        .apply_operations(
            "transition.gz2e01.file-select-horse-name-entry-ready",
            "snapshot.horse-name-ready-again",
            &transition("transition.gz2e01.file-select-horse-name-entry-ready")
                .activation
                .effects,
        )
        .unwrap();
    let horse_name = StateValue::Bytes(b"Epona!\0".to_vec());
    set_structured_field(
        &mut blank,
        NAME_SCENE_CONTROL_COMPONENT,
        "submitted_name_bytes",
        horse_name.clone(),
    );
    set_structured_field(
        &mut blank,
        NAME_SCENE_CONTROL_COMPONENT,
        "input_result_raw",
        StateValue::Unsigned(2),
    );
    blank
        .apply_operations(
            "transition.gz2e01.file-select-horse-name-confirm",
            "snapshot.name-entry-complete",
            &transition("transition.gz2e01.file-select-horse-name-confirm")
                .activation
                .effects,
        )
        .unwrap();
    assert_eq!(
        fields_for(&blank, PLAYER_INFO_COMPONENT)["horse_name_bytes"],
        horse_name
    );
    assert_eq!(
        fields_for(&blank, NAME_SCENE_CONTROL_COMPONENT)["phase"],
        StateValue::Text("selection_end".into())
    );
    assert!(
        blank.snapshot.environment.physical_slots.is_empty(),
        "name confirmation must not fabricate the later successful save"
    );
    let retained_world_location = blank.snapshot.environment.location.clone();
    assert_eq!(
        classify(
            &blank,
            "transition.gz2e01.file-select-new-file-request-play-scene"
        ),
        TransitionClassification::Executable
    );
    blank
        .apply_operations(
            "transition.gz2e01.file-select-new-file-request-play-scene",
            "snapshot.blank-play-scene-requested",
            &transition("transition.gz2e01.file-select-new-file-request-play-scene")
                .activation
                .effects,
        )
        .unwrap();
    assert_eq!(blank.snapshot.environment.location, retained_world_location);
    assert_eq!(
        blank.snapshot.environment.execution_context,
        ExecutionContext::Process {
            process_name: "PROC_NAME_SCENE".into(),
            pending_world_load: Some(SceneLocation {
                stage: "F_SP108".into(),
                room: 1,
                layer: 13,
                spawn: 21,
            }),
        }
    );

    let mut no_card = make_file_select_state(false);
    set_structured_field(
        &mut no_card,
        NAME_SCENE_CONTROL_COMPONENT,
        "phase",
        StateValue::Text("no_save_prompt".into()),
    );
    set_structured_field(
        &mut no_card,
        NAME_SCENE_CONTROL_COMPONENT,
        "no_save_choice_raw",
        StateValue::Unsigned(1),
    );
    set_structured_field(
        &mut no_card,
        INVENTORY_COMPONENT,
        "rupees",
        StateValue::Unsigned(999),
    );
    no_card.validate().unwrap();
    assert_eq!(
        classify(
            &no_card,
            "transition.gz2e01.file-select-proceed-without-card"
        ),
        TransitionClassification::Executable
    );
    assert_eq!(
        classify(&no_card, "transition.gz2e01.file-select-blank-slot-1"),
        TransitionClassification::GuardBlocked
    );
    let no_card_runtime = no_card.snapshot.environment.active_runtime_file.clone();
    no_card
        .apply_operations(
            "transition.gz2e01.file-select-proceed-without-card",
            "snapshot.no-card-name-entry",
            &transition("transition.gz2e01.file-select-proceed-without-card")
                .activation
                .effects,
        )
        .unwrap();
    assert_eq!(
        no_card.snapshot.environment.active_runtime_file,
        no_card_runtime
    );
    assert!(no_card.snapshot.environment.physical_slots.is_empty());
    assert_eq!(
        fields_for(&no_card, INVENTORY_COMPONENT)["rupees"],
        StateValue::Unsigned(0)
    );
    assert_eq!(
        fields_for(&no_card, RUNTIME_FILE_HEADER_COMPONENT)["new_file_raw"],
        StateValue::Unsigned(0),
        "the no-card path never executes blank-slot mNewFile = 128"
    );
    assert_eq!(
        fields_for(&no_card, RUNTIME_FILE_HEADER_COMPONENT)["no_file_raw"],
        StateValue::Unsigned(1)
    );
    assert_eq!(
        fields_for(&no_card, RUNTIME_FILE_HEADER_COMPONENT)["data_num_raw"],
        StateValue::Unsigned(0)
    );
    assert_eq!(
        fields_for(&no_card, DUNGEON_SIX_SAVE_COMPONENT)["key_count"],
        StateValue::Unsigned(0)
    );
    let StateValue::Bytes(lineup) = &fields_for(&no_card, INVENTORY_COMPONENT)["item_lineup"]
    else {
        unreachable!()
    };
    assert!(lineup.iter().all(|item| *item == ITEM_NONE));
    assert_eq!(
        fields_for(&no_card, ACTIVE_VIBRATION_COMPONENT)["enabled_raw"],
        StateValue::Unsigned(1)
    );
    assert_eq!(
        fields_for(&no_card, SAVE_STAGE_DISPLAY_COMPONENT)["stage"],
        StateValue::Text("F_SP108".into())
    );
    set_structured_field(
        &mut no_card,
        NAME_SCENE_CONTROL_COMPONENT,
        "input_result_raw",
        StateValue::Unsigned(1),
    );
    assert_eq!(
        classify(
            &no_card,
            "transition.gz2e01.file-select-player-name-cancel-to-card-check"
        ),
        TransitionClassification::Executable
    );
    assert_eq!(
        classify(
            &no_card,
            "transition.gz2e01.file-select-player-name-cancel-to-data-select"
        ),
        TransitionClassification::GuardBlocked
    );
    assert_eq!(
            no_card
                .serialized_components
                .keys()
                .filter(|owner| matches!(owner, SerializationOwner::Custom { id } if id.starts_with(FILE_SELECT_BUFFER_OWNER_PREFIX)))
                .count(),
            3,
            "three initialized session buffers must not masquerade as physical slots"
        );

    let unknown_slot_state = make_file_select_state(false);
    let slot_one_available = ValueReference::PhysicalSlotImageAvailable {
        slot: PhysicalSlotId(1),
    };
    let evaluator = PredicateEvaluator::new(
        &unknown_slot_state.snapshot,
        &facts,
        &[],
        &BTreeMap::new(),
        EvidencePolicy::RESEARCH,
    )
    .unwrap();
    assert_eq!(evaluator.resolve_value(&slot_one_available), None);
    let mut explicitly_absent = unknown_slot_state.snapshot.clone();
    explicitly_absent
        .environment
        .physical_slot_observations
        .push(PhysicalSlotObservation {
            slot: PhysicalSlotId(1),
            content_status: CaptureStatus::Absent,
            attached_to_active_runtime: false,
        });
    let evaluator = PredicateEvaluator::new(
        &explicitly_absent,
        &facts,
        &[],
        &BTreeMap::new(),
        EvidencePolicy::RESEARCH,
    )
    .unwrap();
    assert_eq!(
        evaluator.resolve_value(&slot_one_available),
        Some(StateValue::Boolean(false))
    );

    let mut existing = make_file_select_state(true);
    let evaluator = PredicateEvaluator::new(
        &existing.snapshot,
        &facts,
        &[],
        &BTreeMap::new(),
        EvidencePolicy::RESEARCH,
    )
    .unwrap();
    assert_eq!(
        evaluator.resolve_value(&slot_one_available),
        Some(StateValue::Boolean(true))
    );
    set_structured_field(
        &mut existing,
        NAME_SCENE_CONTROL_COMPONENT,
        "selected_entry_kind",
        StateValue::Text("existing".into()),
    );
    set_structured_field(
        &mut existing,
        NAME_SCENE_CONTROL_COMPONENT,
        "selected_index_raw",
        StateValue::Unsigned(0),
    );
    existing.validate().unwrap();
    assert_eq!(
        classify(
            &existing,
            "transition.gz2e01.file-select-open-existing-slot"
        ),
        TransitionClassification::Executable
    );
    existing
        .apply_operations(
            "transition.gz2e01.file-select-open-existing-slot",
            "snapshot.existing-slot-menu",
            &transition("transition.gz2e01.file-select-open-existing-slot")
                .activation
                .effects,
        )
        .unwrap();
    assert_eq!(
        classify(
            &existing,
            "transition.gz2e01.file-select-start-existing-slot-1"
        ),
        TransitionClassification::Executable,
        "the exact post-copy normalization closes the existing-slot Start edge"
    );
    let sealed_digest = existing.snapshot.environment.physical_slots[0].serialized_state_sha256;
    existing
        .apply_operations(
            "transition.gz2e01.file-select-start-existing-slot-1",
            "snapshot.existing-slot-1-loaded",
            &transition("transition.gz2e01.file-select-start-existing-slot-1")
                .activation
                .effects,
        )
        .unwrap();
    assert_eq!(
        existing.snapshot.environment.active_runtime_file.id,
        "file-0.file-select-slot-1"
    );
    assert_eq!(
        existing.snapshot.environment.active_runtime_file.origin,
        RuntimeFileOrigin::LoadedSlot {
            slot: PhysicalSlotId(1)
        }
    );
    assert_eq!(
        existing.snapshot.environment.physical_slots[0].serialized_state_sha256,
        sealed_digest
    );
    assert_eq!(
        fields_for(&existing, INVENTORY_COMPONENT)["life"],
        StateValue::Unsigned(12),
        "the selected sealed image replaces the title initializer payload before the exact post-copy life floor"
    );
    let StateValue::Bytes(items) = &fields_for(&existing, INVENTORY_COMPONENT)["inventory"] else {
        unreachable!()
    };
    assert_eq!(items[9], ITEM_NONE);
    assert_eq!(items[10], ITEM_DOUBLE_CLAWSHOT);
    let StateValue::Bytes(lineup) = &fields_for(&existing, INVENTORY_COMPONENT)["item_lineup"]
    else {
        unreachable!()
    };
    assert_eq!(lineup[0], 10);
    assert!(lineup[1..].iter().all(|item| *item == ITEM_NONE));
    assert_eq!(
        fields_for(&existing, DUNGEON_SIX_SAVE_COMPONENT)["key_count"],
        StateValue::Unsigned(0)
    );
    assert_eq!(
        fields_for(&existing, ACTIVE_VIBRATION_COMPONENT)["enabled_raw"],
        StateValue::Unsigned(1)
    );
    assert_eq!(
        fields_for(&existing, SAVE_STAGE_DISPLAY_COMPONENT)["stage"],
        StateValue::Text("R_SP107".into())
    );
    assert_eq!(
        fields_for(&existing, RUNTIME_FILE_HEADER_COMPONENT)["data_num_raw"],
        StateValue::Unsigned(0)
    );
    assert_eq!(
        fields_for(&existing, PLAYER_INFO_COMPONENT)["player_name_bytes"],
        StateValue::Bytes(b"SlotOne\0".to_vec()),
        "player info must come from the selected sealed save projection"
    );
    assert_eq!(
        fields_for(&existing, NAME_SCENE_CONTROL_COMPONENT)["phase"],
        StateValue::Text("selection_end".into())
    );
    for component_id in [
        TEMPORARY_EVENT_COMPONENT,
        RESTART_COMPONENT,
        RUNTIME_FILE_HEADER_COMPONENT,
    ] {
        assert_eq!(
            component_for(&existing, component_id).serialization_owner,
            SerializationOwner::RuntimeFile {
                runtime_file_id: "file-0.file-select-slot-1".into(),
            }
        );
    }
    let retained_world_location = existing.snapshot.environment.location.clone();
    assert_eq!(
        classify(
            &existing,
            "transition.gz2e01.file-select-existing-file-request-play-scene"
        ),
        TransitionClassification::Executable
    );
    existing
        .apply_operations(
            "transition.gz2e01.file-select-existing-file-request-play-scene",
            "snapshot.existing-play-scene-requested",
            &transition("transition.gz2e01.file-select-existing-file-request-play-scene")
                .activation
                .effects,
        )
        .unwrap();
    assert_eq!(
        existing.snapshot.environment.location,
        retained_world_location
    );
    assert_eq!(
        existing.snapshot.environment.execution_context,
        ExecutionContext::Process {
            process_name: "PROC_NAME_SCENE".into(),
            pending_world_load: Some(SceneLocation {
                stage: "R_SP107".into(),
                room: 3,
                layer: -1,
                spawn: 9,
            }),
        }
    );
}

#[test]
fn successful_save_seals_only_the_selected_slot_and_failure_seals_none() {
    let (content, runtime) = context();
    let catalog = gz2e01_reset_to_opening_mechanics(&content, &runtime).unwrap();
    let facts = FactCatalog {
        schema: FACT_CATALOG_SCHEMA.into(),
        aliases: Vec::new(),
        derived_facts: Vec::new(),
    };
    let transition = |id: &str| {
        catalog
            .transitions
            .iter()
            .find(|transition| transition.id == id)
            .unwrap()
    };
    let classify = |state: &PlannerExecutionState, id: &str| {
        PredicateEvaluator::new(
            &state.snapshot,
            &facts,
            &[],
            &BTreeMap::new(),
            EvidencePolicy::RESEARCH,
        )
        .unwrap()
        .assess_transition(
            transition(id),
            &BTreeSet::new(),
            &BTreeSet::new(),
            FeasibilityMode::Modeled,
        )
        .classification
    };

    let mut before = snapshot(runtime);
    let persistent_events = before
        .environment
        .components
        .iter_mut()
        .find(|component| component.id == PERSISTENT_EVENT_COMPONENT)
        .unwrap();
    let ComponentPayload::Raw { bytes, .. } = &mut persistent_events.payload else {
        unreachable!()
    };
    bytes[0x1b] = 0;
    before
        .environment
        .components
        .push(save_menu_control(1, 1, 1, 0));
    before
        .environment
        .components
        .sort_by(|left, right| left.id.cmp(&right.id));
    let mut success = PlannerExecutionState::new(before).unwrap();
    assert_eq!(
        classify(
            &success,
            "transition.gz2e01.save-menu-complete-slot-2-continue"
        ),
        TransitionClassification::Executable
    );
    assert_eq!(
        classify(
            &success,
            "transition.gz2e01.save-menu-complete-slot-1-continue"
        ),
        TransitionClassification::GuardBlocked
    );
    assert_eq!(
        classify(
            &success,
            "transition.gz2e01.save-menu-complete-slot-2-event"
        ),
        TransitionClassification::GuardBlocked
    );
    assert_eq!(
        classify(&success, "transition.gz2e01.save-menu-write-failed"),
        TransitionClassification::GuardBlocked
    );
    let active_runtime = success.snapshot.environment.active_runtime_file.clone();
    success
        .apply_operations(
            "transition.gz2e01.save-menu-complete-slot-2-continue",
            "snapshot.save-slot-2-complete",
            &transition("transition.gz2e01.save-menu-complete-slot-2-continue")
                .activation
                .effects,
        )
        .unwrap();
    assert_eq!(
        success.snapshot.environment.active_runtime_file, active_runtime,
        "saving does not end or replace the live runtime lifetime"
    );
    assert_eq!(success.snapshot.environment.physical_slots.len(), 1);
    assert_eq!(
        success.snapshot.environment.physical_slots[0].slot,
        PhysicalSlotId(2)
    );
    assert_eq!(
        success.snapshot.environment.physical_slots[0].persistent_file_id,
        "file-0.save-slot-2"
    );
    let image = &success.persistent_file_images["file-0.save-slot-2"];
    assert!(
        image
            .runtime_components
            .iter()
            .any(|component| component.id == PLAYER_INFO_COMPONENT)
    );
    assert_eq!(image.stage_banks.len(), 1);
    assert!(matches!(
        &image.stage_banks[0].owner,
        SerializationOwner::StageBank { runtime_file_id, stage }
            if runtime_file_id == "file-0.save-slot-2" && stage == "R_SP107"
    ));
    assert_eq!(
        fields_for(&success, RUNTIME_FILE_HEADER_COMPONENT)["data_num_raw"],
        StateValue::Unsigned(1)
    );
    assert_eq!(
        fields_for(&success, RUNTIME_FILE_HEADER_COMPONENT)["no_file_raw"],
        StateValue::Unsigned(0)
    );
    assert_eq!(
        fields_for(&success, SAVE_MENU_CONTROL_COMPONENT)["phase"],
        StateValue::Text("game_continue_disp".into())
    );

    let mut failed_before = snapshot(context().1);
    failed_before
        .environment
        .components
        .push(save_menu_control(1, 2, 1, 0));
    failed_before
        .environment
        .components
        .sort_by(|left, right| left.id.cmp(&right.id));
    let mut failed = PlannerExecutionState::new(failed_before).unwrap();
    assert_eq!(
        classify(&failed, "transition.gz2e01.save-menu-write-failed"),
        TransitionClassification::Executable
    );
    failed
        .apply_operations(
            "transition.gz2e01.save-menu-write-failed",
            "snapshot.save-failed",
            &transition("transition.gz2e01.save-menu-write-failed")
                .activation
                .effects,
        )
        .unwrap();
    assert!(failed.snapshot.environment.physical_slots.is_empty());
    assert!(failed.persistent_file_images.is_empty());
    assert_eq!(
        fields_for(&failed, RUNTIME_FILE_HEADER_COMPONENT)["data_num_raw"],
        StateValue::Unsigned(3),
        "failed SaveSync must not claim the selected slot"
    );
    assert_eq!(
        fields_for(&failed, SAVE_MENU_CONTROL_COMPONENT)["phase"],
        StateValue::Text("memcard_command_end2".into())
    );
}

#[test]
fn successful_save_projects_lantern_repairs_without_mutating_the_live_runtime() {
    let (content, runtime) = context();
    let catalog = gz2e01_reset_to_opening_mechanics(&content, &runtime).unwrap();
    let facts = FactCatalog {
        schema: FACT_CATALOG_SCHEMA.into(),
        aliases: Vec::new(),
        derived_facts: Vec::new(),
    };
    let transition_id =
        "transition.gz2e01.save-menu-complete-slot-3-continue-event-clear-lantern-restore";
    let transition = catalog
        .transitions
        .iter()
        .find(|transition| transition.id == transition_id)
        .unwrap();

    let mut before = snapshot(runtime);
    let persistent_events = before
        .environment
        .components
        .iter_mut()
        .find(|component| component.id == PERSISTENT_EVENT_COMPONENT)
        .unwrap();
    let ComponentPayload::Raw { bytes, .. } = &mut persistent_events.payload else {
        unreachable!()
    };
    bytes[0x1b] = 0xa0;
    let inventory = before
        .environment
        .components
        .iter_mut()
        .find(|component| component.id == INVENTORY_COMPONENT)
        .unwrap();
    let ComponentPayload::Structured { fields } = &mut inventory.payload else {
        unreachable!()
    };
    let StateValue::Bytes(acquired) = fields.get_mut("acquired_item_bits").unwrap() else {
        unreachable!()
    };
    acquired[9] |= 1;
    fields.insert("oil".into(), StateValue::Unsigned(77));
    before
        .environment
        .components
        .push(save_menu_control(2, 1, 1, 4_321));
    before
        .environment
        .components
        .sort_by(|left, right| left.id.cmp(&right.id));

    let mut state = PlannerExecutionState::new(before).unwrap();
    let classification = PredicateEvaluator::new(
        &state.snapshot,
        &facts,
        &[],
        &BTreeMap::new(),
        EvidencePolicy::RESEARCH,
    )
    .unwrap()
    .assess_transition(
        transition,
        &BTreeSet::new(),
        &BTreeSet::new(),
        FeasibilityMode::Modeled,
    )
    .classification;
    assert_eq!(classification, TransitionClassification::Executable);

    state
        .apply_operations(
            transition_id,
            "snapshot.transformed-save",
            &transition.activation.effects,
        )
        .unwrap();

    let live_events = state
        .snapshot
        .environment
        .components
        .iter()
        .find(|component| component.id == PERSISTENT_EVENT_COMPONENT)
        .unwrap();
    let ComponentPayload::Raw { bytes, .. } = &live_events.payload else {
        unreachable!()
    };
    assert_eq!(bytes[0x1b], 0xa0);
    let live_inventory = fields_for(&state, INVENTORY_COMPONENT);
    let StateValue::Bytes(live_items) = &live_inventory["inventory"] else {
        unreachable!()
    };
    assert_eq!(live_items[1], ITEM_NONE);
    assert_eq!(live_inventory["oil"], StateValue::Unsigned(77));
    let live_player_info = fields_for(&state, PLAYER_INFO_COMPONENT);
    assert!(!live_player_info.contains_key("total_time_ticks"));
    assert!(!live_player_info.contains_key("date_ipl_ticks"));

    let image = &state.persistent_file_images["file-0.save-slot-3"];
    let saved_events = image
        .runtime_components
        .iter()
        .find(|component| component.id == PERSISTENT_EVENT_COMPONENT)
        .unwrap();
    let ComponentPayload::Raw { bytes, .. } = &saved_events.payload else {
        unreachable!()
    };
    assert_eq!(bytes[0x1b], 0x80);
    let saved_inventory = image
        .runtime_components
        .iter()
        .find(|component| component.id == INVENTORY_COMPONENT)
        .unwrap();
    let ComponentPayload::Structured { fields } = &saved_inventory.payload else {
        unreachable!()
    };
    let StateValue::Bytes(saved_items) = &fields["inventory"] else {
        unreachable!()
    };
    assert_eq!(saved_items[1], 0x48);
    assert_eq!(fields["oil"], StateValue::Unsigned(4_321));
    let StateValue::Bytes(saved_acquired) = &fields["acquired_item_bits"] else {
        unreachable!()
    };
    assert_eq!(saved_acquired[9] & 1, 1);
    let saved_player_info = image
        .runtime_components
        .iter()
        .find(|component| component.id == PLAYER_INFO_COMPONENT)
        .unwrap();
    let ComponentPayload::Structured { fields } = &saved_player_info.payload else {
        unreachable!()
    };
    assert!(!fields.contains_key("total_time_ticks"));
    assert!(!fields.contains_key("date_ipl_ticks"));
}

#[test]
fn reset_prefix_is_guard_blocked_when_fader_is_busy() {
    let (content, runtime) = context();
    let catalog = gz2e01_reset_to_opening_mechanics(&content, &runtime).unwrap();
    let mut before = snapshot(runtime);
    let reset_control = before
        .environment
        .components
        .iter_mut()
        .find(|component| component.id == RESET_CONTROL_COMPONENT)
        .unwrap();
    let ComponentPayload::Structured { fields } = &mut reset_control.payload else {
        unreachable!()
    };
    fields.insert("fader_status".into(), StateValue::Unsigned(2));
    let facts = FactCatalog {
        schema: FACT_CATALOG_SCHEMA.into(),
        aliases: Vec::new(),
        derived_facts: Vec::new(),
    };
    let evaluator = PredicateEvaluator::new(
        &before,
        &facts,
        &[],
        &BTreeMap::new(),
        EvidencePolicy::RESEARCH,
    )
    .unwrap();
    assert_eq!(
        evaluator
            .assess_transition(
                catalog
                    .transitions
                    .iter()
                    .find(|transition| transition.id == "transition.gz2e01.reset-to-opening")
                    .unwrap(),
                &BTreeSet::new(),
                &BTreeSet::new(),
                FeasibilityMode::Modeled,
            )
            .classification,
        TransitionClassification::GuardBlocked
    );
}
