//! Assemble the exact reset, title, file-select, save, and play-scene catalog.

use super::*;

pub fn gz2e01_reset_to_opening_mechanics(
    content: &ContentIdentity,
    runtime: &RuntimeConfiguration,
) -> Result<MechanicsCatalog, PlannerContractError> {
    content.validate()?;
    runtime.validate()?;
    let content_sha256 = content.digest()?;
    let runtime_sha256 = runtime.digest()?;
    if content_sha256 != GZ2E01_CONTENT_SHA256
        || runtime_sha256 != GZ2E01_EN_RUNTIME_SHA256
        || runtime.content_sha256 != content_sha256
    {
        return Err(PlannerContractError::new(
            "title_boundary.identity",
            "requires the exact GZ2E01/English context",
        ));
    }

    let scope = ContextScope {
        selectors: vec![ContextSelector::Exact {
            context: ExactContext {
                content_sha256,
                runtime_configuration_sha256: runtime_sha256,
            },
        }],
    };
    let evidence = reset_rule_evidence();
    let compare = |left: ValueReference, operator, value| PredicateExpression::Compare {
        left,
        operator,
        right: ValueReference::Literal { value },
    };
    let control_field = |field: &str| ValueReference::ComponentField {
        component_id: RESET_CONTROL_COMPONENT.into(),
        field: field.into(),
    };
    let reset_transition = CandidateTransition {
        id: "transition.gz2e01.reset-to-opening".into(),
        label: "Reset the active play scene to the opening/title process".into(),
        scope,
        transition_kind: TransitionKind::TitleReturn,
        approach_id: "system-reset.gcn".into(),
        activation: ActivationContract {
            hard_guards: PredicateExpression::All {
                terms: vec![
                    compare(
                        control_field("reset_requested"),
                        ComparisonOperator::Equal,
                        StateValue::Boolean(true),
                    ),
                    compare(
                        control_field("return_to_menu"),
                        ComparisonOperator::Equal,
                        StateValue::Boolean(false),
                    ),
                    compare(
                        control_field("fader_status"),
                        ComparisonOperator::NotEqual,
                        StateValue::Unsigned(2),
                    ),
                ],
            },
            physical_obligation_ids: Vec::new(),
            effects: vec![
                StateOperation::Write {
                    target: ComponentFieldTarget {
                        component_id: RESTART_COMPONENT.into(),
                        field: "room_param".into(),
                    },
                    value: StateValue::Unsigned(0),
                },
                StateOperation::SetExecutionContext {
                    context: ExecutionContext::Process {
                        process_name: "PROC_OPENING_SCENE".into(),
                        pending_world_load: Some(SceneLocation {
                            stage: "F_SP102".into(),
                            room: 0,
                            layer: 10,
                            spawn: 100,
                        }),
                    },
                },
            ],
            unknown_requirements: Vec::new(),
        },
        evidence,
    };
    let scheduler_evidence = scheduler_rule_evidence();
    let opening_process_activation_transition = opening_process_activation_transition(
        reset_transition.scope.clone(),
        scheduler_evidence.clone(),
    );
    let opening_evidence = opening_rule_evidence();
    let pending_compare = |left: ValueReference, value| PredicateExpression::Compare {
        left,
        operator: ComparisonOperator::Equal,
        right: ValueReference::Literal { value },
    };
    let mut opening_effects = dcomifgs_init_effects();
    opening_effects.extend([
        StateOperation::WriteRaw {
            component_id: PERSISTENT_EVENT_COMPONENT.into(),
            byte_offset: 6,
            mask: vec![1],
            value: vec![1],
        },
        StateOperation::WriteFields {
            component_id: INVENTORY_COMPONENT.into(),
            fields: BTreeMap::from([
                ("maximum_life".into(), StateValue::Unsigned(15)),
                ("life".into(), StateValue::Unsigned(12)),
                (
                    "equipment".into(),
                    StateValue::Bytes(vec![0x2f, 0x28, 0x2c, 0xff, 0xff, 0]),
                ),
                (
                    "collect_item_bits".into(),
                    StateValue::Bytes(vec![0, 1, 4, 0, 0, 0, 0, 0]),
                ),
            ]),
        },
        StateOperation::CompletePendingWorldLoad,
        StateOperation::Write {
            target: ComponentFieldTarget {
                component_id: OPENING_PROCESS_CONTROL_COMPONENT.into(),
                field: "phase".into(),
            },
            value: StateValue::Text("complete".into()),
        },
    ]);
    let opening_process_guards = vec![
        pending_compare(
            ValueReference::ExecutionProcess,
            StateValue::Text("PROC_OPENING_SCENE".into()),
        ),
        pending_compare(
            ValueReference::PendingWorldLoadStage,
            StateValue::Text("F_SP102".into()),
        ),
        pending_compare(ValueReference::PendingWorldLoadRoom, StateValue::Signed(0)),
        pending_compare(
            ValueReference::PendingWorldLoadLayer,
            StateValue::Signed(10),
        ),
        pending_compare(
            ValueReference::PendingWorldLoadSpawn,
            StateValue::Signed(100),
        ),
        pending_compare(
            ValueReference::ComponentField {
                component_id: OPENING_PROCESS_CONTROL_COMPONENT.into(),
                field: "phase".into(),
            },
            StateValue::Text("phase_4".into()),
        ),
    ];
    let title_file_0_guard = pending_compare(
        ValueReference::ActiveRuntimeFileOrigin,
        StateValue::Text("title_file_0".into()),
    );
    let mut enter_and_initialize_effects = vec![StateOperation::BeginRuntimeFileLifetime {
        destination_id_suffix: "title-file-0".into(),
        origin: RuntimeFileOrigin::TitleFile0,
        backing: BackingAttachment::MemoryOnly,
        allowed_serialization_targets: vec![
            PhysicalSlotId(1),
            PhysicalSlotId(2),
            PhysicalSlotId(3),
        ],
    }];
    enter_and_initialize_effects.extend(opening_effects.clone());
    let enter_and_initialize_transition = CandidateTransition {
        id: "transition.gz2e01.opening-enter-and-initialize-file0".into(),
        label: "Begin title-origin file 0 and run opening phase 4".into(),
        scope: reset_transition.scope.clone(),
        transition_kind: TransitionKind::TitleReturn,
        approach_id: "process.opening-scene.phase-4.new-runtime".into(),
        activation: ActivationContract {
            hard_guards: PredicateExpression::All {
                terms: std::iter::once(PredicateExpression::Not {
                    term: Box::new(title_file_0_guard.clone()),
                })
                .chain(opening_process_guards.iter().cloned())
                .collect(),
            },
            physical_obligation_ids: Vec::new(),
            effects: enter_and_initialize_effects,
            unknown_requirements: Vec::new(),
        },
        evidence: opening_evidence.clone(),
    };
    let opening_transition = CandidateTransition {
        id: "transition.gz2e01.opening-file0-initialize".into(),
        label: "Run opening phase 4 and initialize title-origin file 0".into(),
        scope: reset_transition.scope.clone(),
        transition_kind: TransitionKind::TitleReturn,
        approach_id: "process.opening-scene.phase-4".into(),
        activation: ActivationContract {
            hard_guards: PredicateExpression::All {
                terms: std::iter::once(title_file_0_guard.clone())
                    .chain(opening_process_guards)
                    .collect(),
            },
            physical_obligation_ids: Vec::new(),
            effects: opening_effects,
            unknown_requirements: Vec::new(),
        },
        evidence: opening_evidence,
    };
    let title_evidence = title_rule_evidence();
    let title_field = |field: &str| ValueReference::ComponentField {
        component_id: TITLE_CONTROL_COMPONENT.into(),
        field: field.into(),
    };
    let title_key_accept_transition = CandidateTransition {
        id: "transition.gz2e01.title-key-accept".into(),
        label: "Accept A or Start at the title prompt".into(),
        scope: reset_transition.scope.clone(),
        transition_kind: TransitionKind::Other,
        approach_id: "title.input.key-wait".into(),
        activation: ActivationContract {
            hard_guards: PredicateExpression::All {
                terms: vec![
                    pending_compare(
                        ValueReference::ExecutionProcess,
                        StateValue::Text("PROC_OPENING_SCENE".into()),
                    ),
                    pending_compare(
                        ValueReference::ComponentField {
                            component_id: OPENING_PROCESS_CONTROL_COMPONENT.into(),
                            field: "phase".into(),
                        },
                        StateValue::Text("complete".into()),
                    ),
                    pending_compare(title_field("phase"), StateValue::Text("key_wait".into())),
                    pending_compare(title_field("reset_requested"), StateValue::Boolean(false)),
                    pending_compare(title_field("overlap_peek"), StateValue::Boolean(false)),
                    PredicateExpression::Any {
                        terms: vec![
                            pending_compare(title_field("a_triggered"), StateValue::Boolean(true)),
                            pending_compare(
                                title_field("start_triggered"),
                                StateValue::Boolean(true),
                            ),
                        ],
                    },
                ],
            },
            physical_obligation_ids: Vec::new(),
            effects: vec![StateOperation::Write {
                target: ComponentFieldTarget {
                    component_id: TITLE_CONTROL_COMPONENT.into(),
                    field: "phase".into(),
                },
                value: StateValue::Text("next_scene".into()),
            }],
            unknown_requirements: Vec::new(),
        },
        evidence: title_evidence.clone(),
    };
    let title_request_name_scene_transition = CandidateTransition {
        id: "transition.gz2e01.title-request-name-scene".into(),
        label: "Request the normal name and file-select scene".into(),
        scope: reset_transition.scope.clone(),
        transition_kind: TransitionKind::ActorDriven,
        approach_id: "title.next-scene.normal".into(),
        activation: ActivationContract {
            hard_guards: PredicateExpression::All {
                terms: vec![
                    pending_compare(
                        ValueReference::ExecutionProcess,
                        StateValue::Text("PROC_OPENING_SCENE".into()),
                    ),
                    pending_compare(title_field("phase"), StateValue::Text("next_scene".into())),
                    pending_compare(title_field("reset_requested"), StateValue::Boolean(false)),
                    pending_compare(title_field("overlap_peek"), StateValue::Boolean(false)),
                ],
            },
            physical_obligation_ids: Vec::new(),
            // `fopScnM_ChangeReq` submits a process change. It does not prove
            // that the process manager has destroyed the opening process or
            // completed `dScnName_c::create`, so retain the active process and
            // record only the request here. A later observed NAME_SCENE process
            // and create phase authorize the file-select initializer below.
            effects: vec![StateOperation::Write {
                target: ComponentFieldTarget {
                    component_id: TITLE_CONTROL_COMPONENT.into(),
                    field: "phase".into(),
                },
                value: StateValue::Text("scene_requested".into()),
            }],
            unknown_requirements: Vec::new(),
        },
        evidence: title_evidence,
    };
    let name_scene_activation_transition =
        name_scene_activation_transition(reset_transition.scope.clone(), scheduler_evidence);
    let file_select_evidence = file_select_rule_evidence();
    let mut file_select_create_effects = dcomifgs_init_effects();
    file_select_create_effects.extend([
        StateOperation::Write {
            target: ComponentFieldTarget {
                component_id: RUNTIME_FILE_HEADER_COMPONENT.into(),
                field: "new_file_raw".into(),
            },
            value: StateValue::Unsigned(0),
        },
        StateOperation::Write {
            target: ComponentFieldTarget {
                component_id: RUNTIME_FILE_HEADER_COMPONENT.into(),
                field: "no_file_raw".into(),
            },
            value: StateValue::Unsigned(0),
        },
        StateOperation::Write {
            target: ComponentFieldTarget {
                component_id: NAME_SCENE_CONTROL_COMPONENT.into(),
                field: "phase".into(),
            },
            value: StateValue::Text("file_select_open".into()),
        },
    ]);
    let name_scene_file_select_transition = CandidateTransition {
        id: "transition.gz2e01.name-scene-file-select-initialize".into(),
        label: "Construct file select and reset its title-file-0 save image".into(),
        scope: reset_transition.scope.clone(),
        transition_kind: TransitionKind::Other,
        approach_id: "name-scene.create.file-select".into(),
        activation: ActivationContract {
            hard_guards: PredicateExpression::All {
                terms: vec![
                    title_file_0_guard,
                    pending_compare(
                        ValueReference::ExecutionProcess,
                        StateValue::Text("PROC_NAME_SCENE".into()),
                    ),
                    pending_compare(
                        ValueReference::ComponentField {
                            component_id: NAME_SCENE_CONTROL_COMPONENT.into(),
                            field: "phase".into(),
                        },
                        StateValue::Text("create_file_select".into()),
                    ),
                ],
            },
            physical_obligation_ids: Vec::new(),
            effects: file_select_create_effects,
            unknown_requirements: Vec::new(),
        },
        evidence: file_select_evidence,
    };
    let file_select_branch_evidence = file_select_branch_rule_evidence();
    let name_field = |field: &str| ValueReference::ComponentField {
        component_id: NAME_SCENE_CONTROL_COMPONENT.into(),
        field: field.into(),
    };
    let name_process_guard = pending_compare(
        ValueReference::ExecutionProcess,
        StateValue::Text("PROC_NAME_SCENE".into()),
    );
    let selected_index_guard = |index: u64| {
        pending_compare(
            name_field("selected_index_raw"),
            StateValue::Unsigned(index),
        )
    };
    let mut file_select_branch_transitions = Vec::new();
    for index in 0_u64..3 {
        let slot = index + 1;
        file_select_branch_transitions.push(CandidateTransition {
            id: format!("transition.gz2e01.file-select-focus-blank-slot-{slot}"),
            label: format!("Focus blank save slot {slot}"),
            scope: reset_transition.scope.clone(),
            transition_kind: TransitionKind::Other,
            approach_id: format!("file-select.focus-blank-slot-{slot}"),
            activation: ActivationContract {
                hard_guards: PredicateExpression::All {
                    terms: vec![
                        name_process_guard.clone(),
                        pending_compare(
                            name_field("phase"),
                            StateValue::Text("file_select_open".into()),
                        ),
                    ],
                },
                physical_obligation_ids: Vec::new(),
                effects: vec![StateOperation::WriteFields {
                    component_id: NAME_SCENE_CONTROL_COMPONENT.into(),
                    fields: BTreeMap::from([
                        ("selected_entry_kind".into(), StateValue::Text("new".into())),
                        ("selected_index_raw".into(), StateValue::Unsigned(index)),
                    ]),
                }],
                unknown_requirements: Vec::new(),
            },
            evidence: file_select_branch_evidence.clone(),
        });
        file_select_branch_transitions.push(CandidateTransition {
            id: format!("transition.gz2e01.file-select-blank-slot-{slot}"),
            label: format!("Select blank save slot {slot}"),
            scope: reset_transition.scope.clone(),
            transition_kind: TransitionKind::Other,
            approach_id: format!("file-select.blank-slot-{slot}"),
            activation: ActivationContract {
                hard_guards: PredicateExpression::All {
                    terms: vec![
                        name_process_guard.clone(),
                        pending_compare(
                            name_field("phase"),
                            StateValue::Text("file_select_open".into()),
                        ),
                        pending_compare(
                            name_field("selected_entry_kind"),
                            StateValue::Text("new".into()),
                        ),
                        selected_index_guard(index),
                    ],
                },
                physical_obligation_ids: Vec::new(),
                effects: vec![
                    StateOperation::Write {
                        target: ComponentFieldTarget {
                            component_id: RUNTIME_FILE_HEADER_COMPONENT.into(),
                            field: "new_file_raw".into(),
                        },
                        value: StateValue::Unsigned(128),
                    },
                    StateOperation::Write {
                        target: ComponentFieldTarget {
                            component_id: RUNTIME_FILE_HEADER_COMPONENT.into(),
                            field: "data_num_raw".into(),
                        },
                        value: StateValue::Unsigned(index),
                    },
                    StateOperation::Write {
                        target: ComponentFieldTarget {
                            component_id: PLAYER_INFO_COMPONENT.into(),
                            field: "player_name_bytes".into(),
                        },
                        value: StateValue::Bytes(DEFAULT_PLAYER_NAME_BYTES.to_vec()),
                    },
                    StateOperation::Write {
                        target: ComponentFieldTarget {
                            component_id: NAME_SCENE_CONTROL_COMPONENT.into(),
                            field: "phase".into(),
                        },
                        value: StateValue::Text("name_entry".into()),
                    },
                ],
                unknown_requirements: Vec::new(),
            },
            evidence: file_select_branch_evidence.clone(),
        });
    }
    file_select_branch_transitions.push(CandidateTransition {
        id: "transition.gz2e01.file-select-open-existing-slot".into(),
        label: "Open the selected existing-slot command menu".into(),
        scope: reset_transition.scope.clone(),
        transition_kind: TransitionKind::Other,
        approach_id: "file-select.existing-slot-menu".into(),
        activation: ActivationContract {
            hard_guards: PredicateExpression::All {
                terms: vec![
                    name_process_guard.clone(),
                    pending_compare(
                        name_field("phase"),
                        StateValue::Text("file_select_open".into()),
                    ),
                    pending_compare(
                        name_field("selected_entry_kind"),
                        StateValue::Text("existing".into()),
                    ),
                    PredicateExpression::Any {
                        terms: (0_u64..3).map(selected_index_guard).collect(),
                    },
                ],
            },
            physical_obligation_ids: Vec::new(),
            effects: vec![
                StateOperation::Write {
                    target: ComponentFieldTarget {
                        component_id: RUNTIME_FILE_HEADER_COMPONENT.into(),
                        field: "new_file_raw".into(),
                    },
                    value: StateValue::Unsigned(0),
                },
                StateOperation::Write {
                    target: ComponentFieldTarget {
                        component_id: NAME_SCENE_CONTROL_COMPONENT.into(),
                        field: "menu_command_raw".into(),
                    },
                    value: StateValue::Unsigned(1),
                },
                StateOperation::Write {
                    target: ComponentFieldTarget {
                        component_id: NAME_SCENE_CONTROL_COMPONENT.into(),
                        field: "phase".into(),
                    },
                    value: StateValue::Text("existing_slot_menu".into()),
                },
            ],
            unknown_requirements: Vec::new(),
        },
        evidence: file_select_branch_evidence.clone(),
    });
    let carried_runtime_component_ids = vec![
        TEMPORARY_EVENT_COMPONENT.into(),
        RESTART_COMPONENT.into(),
        RUNTIME_FILE_HEADER_COMPONENT.into(),
    ];
    for index in 0_u64..3 {
        let slot = index + 1;
        let mut effects = vec![StateOperation::LoadActiveRuntimeFromSlot {
            source_slot: PhysicalSlotId(slot as u8),
            destination_id_suffix: format!("file-select-slot-{slot}"),
            destination_allowed_serialization_targets: vec![
                PhysicalSlotId(1),
                PhysicalSlotId(2),
                PhysicalSlotId(3),
            ],
            carried_runtime_component_ids: carried_runtime_component_ids.clone(),
        }];
        effects.extend(file_select_post_copy_normalization());
        effects.extend([
            StateOperation::Write {
                target: ComponentFieldTarget {
                    component_id: RUNTIME_FILE_HEADER_COMPONENT.into(),
                    field: "data_num_raw".into(),
                },
                value: StateValue::Unsigned(index),
            },
            StateOperation::Write {
                target: ComponentFieldTarget {
                    component_id: NAME_SCENE_CONTROL_COMPONENT.into(),
                    field: "phase".into(),
                },
                value: StateValue::Text("selection_end".into()),
            },
        ]);
        file_select_branch_transitions.push(CandidateTransition {
            id: format!("transition.gz2e01.file-select-start-existing-slot-{slot}"),
            label: format!("Load and start existing save slot {slot}"),
            scope: reset_transition.scope.clone(),
            transition_kind: TransitionKind::Other,
            approach_id: format!("file-select.start-existing-slot-{slot}"),
            activation: ActivationContract {
                hard_guards: PredicateExpression::All {
                    terms: vec![
                        name_process_guard.clone(),
                        pending_compare(
                            name_field("phase"),
                            StateValue::Text("existing_slot_menu".into()),
                        ),
                        pending_compare(
                            name_field("selected_entry_kind"),
                            StateValue::Text("existing".into()),
                        ),
                        selected_index_guard(index),
                        pending_compare(name_field("menu_command_raw"), StateValue::Unsigned(1)),
                        pending_compare(
                            ValueReference::PhysicalSlotImageAvailable {
                                slot: PhysicalSlotId(slot as u8),
                            },
                            StateValue::Boolean(true),
                        ),
                    ],
                },
                physical_obligation_ids: Vec::new(),
                effects,
                unknown_requirements: Vec::new(),
            },
            evidence: file_select_branch_evidence.clone(),
        });
    }
    let initialized_buffer_component_ids = vec![
        PERSISTENT_EVENT_COMPONENT.into(),
        INVENTORY_COMPONENT.into(),
        RETURN_PLACE_COMPONENT.into(),
        DUNGEON_SIX_SAVE_COMPONENT.into(),
        PLAYER_INFO_COMPONENT.into(),
        LIGHT_DROP_COMPONENT.into(),
    ];
    let mut no_card_effects = (1_u8..=3)
        .map(|slot| StateOperation::ReplaceCustomStore {
            owner: file_select_buffer_owner(slot),
            components: initialized_file_select_buffer(slot),
        })
        .collect::<Vec<_>>();
    no_card_effects.push(StateOperation::RestorePayloadsFromCustomStore {
        owner: file_select_buffer_owner(1),
        component_ids: initialized_buffer_component_ids,
    });
    no_card_effects.push(StateOperation::WriteFields {
        component_id: PLAYER_INFO_COMPONENT.into(),
        fields: BTreeMap::from([
            (
                "horse_name_bytes".into(),
                StateValue::Bytes(DEFAULT_HORSE_NAME_BYTES.to_vec()),
            ),
            (
                "player_name_bytes".into(),
                StateValue::Bytes(DEFAULT_PLAYER_NAME_BYTES.to_vec()),
            ),
        ]),
    });
    no_card_effects.extend(file_select_post_copy_normalization());
    no_card_effects.extend([
        StateOperation::WriteFields {
            component_id: RUNTIME_FILE_HEADER_COMPONENT.into(),
            fields: BTreeMap::from([
                ("no_file_raw".into(), StateValue::Unsigned(1)),
                ("data_num_raw".into(), StateValue::Unsigned(0)),
            ]),
        },
        StateOperation::InvalidateActiveRuntimeSerializedPayloads {
            selector: ComponentSelector::Kind {
                component_kind: ComponentKind::DungeonMemory,
            },
        },
        StateOperation::ReplacePayload {
            component_id: OBSERVED_EVENT_COMPONENT.into(),
            payload: ComponentPayload::Unknown {
                expected_bytes: None,
            },
        },
        StateOperation::Write {
            target: ComponentFieldTarget {
                component_id: NAME_SCENE_CONTROL_COMPONENT.into(),
                field: "entry_kinds_raw".into(),
            },
            value: StateValue::Bytes(vec![1, 1, 1]),
        },
        StateOperation::Write {
            target: ComponentFieldTarget {
                component_id: NAME_SCENE_CONTROL_COMPONENT.into(),
                field: "phase".into(),
            },
            value: StateValue::Text("name_entry".into()),
        },
    ]);
    file_select_branch_transitions.push(CandidateTransition {
        id: "transition.gz2e01.file-select-proceed-without-card".into(),
        label: "Initialize memory-only save buffers and proceed without a card".into(),
        scope: reset_transition.scope.clone(),
        transition_kind: TransitionKind::Other,
        approach_id: "file-select.no-card-proceed".into(),
        activation: ActivationContract {
            hard_guards: PredicateExpression::All {
                terms: vec![
                    name_process_guard.clone(),
                    pending_compare(
                        name_field("phase"),
                        StateValue::Text("no_save_prompt".into()),
                    ),
                    pending_compare(name_field("no_save_choice_raw"), StateValue::Unsigned(1)),
                ],
            },
            physical_obligation_ids: Vec::new(),
            effects: no_card_effects,
            unknown_requirements: Vec::new(),
        },
        evidence: file_select_branch_evidence.clone(),
    });
    let name_confirmation_evidence = name_confirmation_rule_evidence();
    let runtime_header_field = |field: &str| ValueReference::ComponentField {
        component_id: RUNTIME_FILE_HEADER_COMPONENT.into(),
        field: field.into(),
    };
    file_select_branch_transitions.extend([
        CandidateTransition {
            id: "transition.gz2e01.file-select-player-name-confirm".into(),
            label: "Confirm the new file's player name".into(),
            scope: reset_transition.scope.clone(),
            transition_kind: TransitionKind::Other,
            approach_id: "file-select.name-entry.player.confirm".into(),
            activation: ActivationContract {
                hard_guards: PredicateExpression::All {
                    terms: vec![
                        name_process_guard.clone(),
                        pending_compare(name_field("phase"), StateValue::Text("name_entry".into())),
                        pending_compare(name_field("input_result_raw"), StateValue::Unsigned(2)),
                    ],
                },
                physical_obligation_ids: Vec::new(),
                effects: vec![
                    StateOperation::CopyValue {
                        source: ComponentFieldTarget {
                            component_id: NAME_SCENE_CONTROL_COMPONENT.into(),
                            field: "submitted_name_bytes".into(),
                        },
                        target: ComponentFieldTarget {
                            component_id: PLAYER_INFO_COMPONENT.into(),
                            field: "player_name_bytes".into(),
                        },
                    },
                    StateOperation::WriteFields {
                        component_id: NAME_SCENE_CONTROL_COMPONENT.into(),
                        fields: BTreeMap::from([
                            ("fade_timer_raw".into(), StateValue::Unsigned(15)),
                            ("phase".into(), StateValue::Text("player_name_fade".into())),
                        ]),
                    },
                ],
                unknown_requirements: Vec::new(),
            },
            evidence: name_confirmation_evidence.clone(),
        },
        CandidateTransition {
            id: "transition.gz2e01.file-select-player-name-cancel-to-data-select".into(),
            label: "Back out of player-name entry to file selection".into(),
            scope: reset_transition.scope.clone(),
            transition_kind: TransitionKind::Other,
            approach_id: "file-select.name-entry.player.back.card".into(),
            activation: ActivationContract {
                hard_guards: PredicateExpression::All {
                    terms: vec![
                        name_process_guard.clone(),
                        pending_compare(name_field("phase"), StateValue::Text("name_entry".into())),
                        pending_compare(name_field("input_result_raw"), StateValue::Unsigned(1)),
                        pending_compare(
                            runtime_header_field("no_file_raw"),
                            StateValue::Unsigned(0),
                        ),
                    ],
                },
                physical_obligation_ids: Vec::new(),
                effects: vec![StateOperation::Write {
                    target: ComponentFieldTarget {
                        component_id: NAME_SCENE_CONTROL_COMPONENT.into(),
                        field: "phase".into(),
                    },
                    value: StateValue::Text("name_to_data_select_move".into()),
                }],
                unknown_requirements: Vec::new(),
            },
            evidence: name_confirmation_evidence.clone(),
        },
        CandidateTransition {
            id: "transition.gz2e01.file-select-player-name-cancel-to-card-check".into(),
            label: "Back out of no-card player-name entry".into(),
            scope: reset_transition.scope.clone(),
            transition_kind: TransitionKind::Other,
            approach_id: "file-select.name-entry.player.back.no-card".into(),
            activation: ActivationContract {
                hard_guards: PredicateExpression::All {
                    terms: vec![
                        name_process_guard.clone(),
                        pending_compare(name_field("phase"), StateValue::Text("name_entry".into())),
                        pending_compare(name_field("input_result_raw"), StateValue::Unsigned(1)),
                        pending_compare(
                            runtime_header_field("no_file_raw"),
                            StateValue::Unsigned(1),
                        ),
                    ],
                },
                physical_obligation_ids: Vec::new(),
                effects: vec![StateOperation::WriteFields {
                    component_id: NAME_SCENE_CONTROL_COMPONENT.into(),
                    fields: BTreeMap::from([
                        (
                            "card_check_phase".into(),
                            StateValue::Text("stat_check".into()),
                        ),
                        ("phase".into(), StateValue::Text("memcard_check".into())),
                    ]),
                }],
                unknown_requirements: Vec::new(),
            },
            evidence: name_confirmation_evidence.clone(),
        },
        CandidateTransition {
            id: "transition.gz2e01.file-select-player-name-fade-complete".into(),
            label: "Initialize the default horse name after player-name fade".into(),
            scope: reset_transition.scope.clone(),
            transition_kind: TransitionKind::Other,
            approach_id: "file-select.name-entry.horse.initialize".into(),
            activation: ActivationContract {
                hard_guards: PredicateExpression::All {
                    terms: vec![
                        name_process_guard.clone(),
                        pending_compare(
                            name_field("phase"),
                            StateValue::Text("player_name_fade".into()),
                        ),
                        pending_compare(name_field("fade_timer_raw"), StateValue::Unsigned(0)),
                    ],
                },
                physical_obligation_ids: Vec::new(),
                effects: vec![
                    StateOperation::Write {
                        target: ComponentFieldTarget {
                            component_id: PLAYER_INFO_COMPONENT.into(),
                            field: "horse_name_bytes".into(),
                        },
                        value: StateValue::Bytes(DEFAULT_HORSE_NAME_BYTES.to_vec()),
                    },
                    StateOperation::WriteFields {
                        component_id: NAME_SCENE_CONTROL_COMPONENT.into(),
                        fields: BTreeMap::from([
                            ("fade_timer_raw".into(), StateValue::Unsigned(15)),
                            ("phase".into(), StateValue::Text("horse_name_move".into())),
                        ]),
                    },
                ],
                unknown_requirements: Vec::new(),
            },
            evidence: name_confirmation_evidence.clone(),
        },
        CandidateTransition {
            id: "transition.gz2e01.file-select-horse-name-entry-ready".into(),
            label: "Finish the fade into horse-name entry".into(),
            scope: reset_transition.scope.clone(),
            transition_kind: TransitionKind::Other,
            approach_id: "file-select.name-entry.horse.ready".into(),
            activation: ActivationContract {
                hard_guards: PredicateExpression::All {
                    terms: vec![
                        name_process_guard.clone(),
                        pending_compare(
                            name_field("phase"),
                            StateValue::Text("horse_name_move".into()),
                        ),
                        pending_compare(name_field("fade_timer_raw"), StateValue::Unsigned(0)),
                        pending_compare(name_field("reset_requested"), StateValue::Boolean(false)),
                    ],
                },
                physical_obligation_ids: Vec::new(),
                effects: vec![StateOperation::Write {
                    target: ComponentFieldTarget {
                        component_id: NAME_SCENE_CONTROL_COMPONENT.into(),
                        field: "phase".into(),
                    },
                    value: StateValue::Text("horse_name_entry".into()),
                }],
                unknown_requirements: Vec::new(),
            },
            evidence: name_confirmation_evidence.clone(),
        },
        CandidateTransition {
            id: "transition.gz2e01.file-select-horse-name-confirm".into(),
            label: "Confirm the horse name and end file selection".into(),
            scope: reset_transition.scope.clone(),
            transition_kind: TransitionKind::Other,
            approach_id: "file-select.name-entry.horse.confirm".into(),
            activation: ActivationContract {
                hard_guards: PredicateExpression::All {
                    terms: vec![
                        name_process_guard.clone(),
                        pending_compare(
                            name_field("phase"),
                            StateValue::Text("horse_name_entry".into()),
                        ),
                        pending_compare(name_field("input_result_raw"), StateValue::Unsigned(2)),
                    ],
                },
                physical_obligation_ids: Vec::new(),
                effects: vec![
                    StateOperation::CopyValue {
                        source: ComponentFieldTarget {
                            component_id: NAME_SCENE_CONTROL_COMPONENT.into(),
                            field: "submitted_name_bytes".into(),
                        },
                        target: ComponentFieldTarget {
                            component_id: PLAYER_INFO_COMPONENT.into(),
                            field: "horse_name_bytes".into(),
                        },
                    },
                    StateOperation::Write {
                        target: ComponentFieldTarget {
                            component_id: NAME_SCENE_CONTROL_COMPONENT.into(),
                            field: "phase".into(),
                        },
                        value: StateValue::Text("selection_end".into()),
                    },
                ],
                unknown_requirements: Vec::new(),
            },
            evidence: name_confirmation_evidence.clone(),
        },
        CandidateTransition {
            id: "transition.gz2e01.file-select-horse-name-back".into(),
            label: "Back from horse-name entry toward player-name entry".into(),
            scope: reset_transition.scope.clone(),
            transition_kind: TransitionKind::Other,
            approach_id: "file-select.name-entry.horse.back".into(),
            activation: ActivationContract {
                hard_guards: PredicateExpression::All {
                    terms: vec![
                        name_process_guard.clone(),
                        pending_compare(
                            name_field("phase"),
                            StateValue::Text("horse_name_entry".into()),
                        ),
                        pending_compare(name_field("input_result_raw"), StateValue::Unsigned(1)),
                    ],
                },
                physical_obligation_ids: Vec::new(),
                effects: vec![StateOperation::WriteFields {
                    component_id: NAME_SCENE_CONTROL_COMPONENT.into(),
                    fields: BTreeMap::from([
                        ("fade_timer_raw".into(), StateValue::Unsigned(15)),
                        (
                            "phase".into(),
                            StateValue::Text("player_name_back_fade".into()),
                        ),
                    ]),
                }],
                unknown_requirements: Vec::new(),
            },
            evidence: name_confirmation_evidence.clone(),
        },
        CandidateTransition {
            id: "transition.gz2e01.file-select-player-name-back-fade-complete".into(),
            label: "Finish the fade back to player-name movement".into(),
            scope: reset_transition.scope.clone(),
            transition_kind: TransitionKind::Other,
            approach_id: "file-select.name-entry.player.back-fade".into(),
            activation: ActivationContract {
                hard_guards: PredicateExpression::All {
                    terms: vec![
                        name_process_guard.clone(),
                        pending_compare(
                            name_field("phase"),
                            StateValue::Text("player_name_back_fade".into()),
                        ),
                        pending_compare(name_field("fade_timer_raw"), StateValue::Unsigned(0)),
                    ],
                },
                physical_obligation_ids: Vec::new(),
                effects: vec![StateOperation::WriteFields {
                    component_id: NAME_SCENE_CONTROL_COMPONENT.into(),
                    fields: BTreeMap::from([
                        ("fade_timer_raw".into(), StateValue::Unsigned(15)),
                        (
                            "phase".into(),
                            StateValue::Text("player_name_back_move".into()),
                        ),
                    ]),
                }],
                unknown_requirements: Vec::new(),
            },
            evidence: name_confirmation_evidence.clone(),
        },
        CandidateTransition {
            id: "transition.gz2e01.file-select-player-name-back-ready".into(),
            label: "Return from horse-name entry to player-name entry".into(),
            scope: reset_transition.scope.clone(),
            transition_kind: TransitionKind::Other,
            approach_id: "file-select.name-entry.player.back-ready".into(),
            activation: ActivationContract {
                hard_guards: PredicateExpression::All {
                    terms: vec![
                        name_process_guard.clone(),
                        pending_compare(
                            name_field("phase"),
                            StateValue::Text("player_name_back_move".into()),
                        ),
                        pending_compare(name_field("fade_timer_raw"), StateValue::Unsigned(0)),
                        pending_compare(name_field("reset_requested"), StateValue::Boolean(false)),
                    ],
                },
                physical_obligation_ids: Vec::new(),
                effects: vec![StateOperation::Write {
                    target: ComponentFieldTarget {
                        component_id: NAME_SCENE_CONTROL_COMPONENT.into(),
                        field: "phase".into(),
                    },
                    value: StateValue::Text("name_entry".into()),
                }],
                unknown_requirements: Vec::new(),
            },
            evidence: name_confirmation_evidence,
        },
    ]);
    let successful_save_evidence = successful_save_rule_evidence();
    let save_field = |field: &str| ValueReference::ComponentField {
        component_id: SAVE_MENU_CONTROL_COMPONENT.into(),
        field: field.into(),
    };
    let identity_lantern_event_projection = PredicateExpression::Any {
        terms: vec![
            pending_compare(
                ValueReference::RawBits {
                    component_id: PERSISTENT_EVENT_COMPONENT.into(),
                    byte_offset: 0x1b,
                    byte_width: 1,
                    mask: 0x08,
                },
                StateValue::Unsigned(0x08),
            ),
            pending_compare(
                ValueReference::RawBits {
                    component_id: PERSISTENT_EVENT_COMPONENT.into(),
                    byte_offset: 0x1b,
                    byte_width: 1,
                    mask: 0x30,
                },
                StateValue::Unsigned(0),
            ),
        ],
    };
    let event_projection_required = PredicateExpression::All {
        terms: vec![
            pending_compare(
                ValueReference::RawBits {
                    component_id: PERSISTENT_EVENT_COMPONENT.into(),
                    byte_offset: 0x1b,
                    byte_width: 1,
                    mask: 0x08,
                },
                StateValue::Unsigned(0),
            ),
            PredicateExpression::Compare {
                left: ValueReference::RawBits {
                    component_id: PERSISTENT_EVENT_COMPONENT.into(),
                    byte_offset: 0x1b,
                    byte_width: 1,
                    mask: 0x30,
                },
                operator: ComparisonOperator::NotEqual,
                right: ValueReference::Literal {
                    value: StateValue::Unsigned(0),
                },
            },
        ],
    };
    let lantern_acquired = ValueReference::ComponentBytes {
        component_id: INVENTORY_COMPONENT.into(),
        field: "acquired_item_bits".into(),
        byte_offset: 9,
        byte_width: 1,
        mask: 0x01,
    };
    let inventory_slot_one = ValueReference::ComponentBytes {
        component_id: INVENTORY_COMPONENT.into(),
        field: "inventory".into(),
        byte_offset: 1,
        byte_width: 1,
        mask: 0xff,
    };
    let identity_lantern_item_projection = PredicateExpression::Any {
        terms: vec![
            pending_compare(lantern_acquired.clone(), StateValue::Unsigned(0)),
            PredicateExpression::Compare {
                left: inventory_slot_one.clone(),
                operator: ComparisonOperator::NotEqual,
                right: ValueReference::Literal {
                    value: StateValue::Unsigned(ITEM_NONE.into()),
                },
            },
        ],
    };
    let lantern_item_projection_required = PredicateExpression::All {
        terms: vec![
            pending_compare(lantern_acquired, StateValue::Unsigned(1)),
            pending_compare(inventory_slot_one, StateValue::Unsigned(ITEM_NONE.into())),
            PredicateExpression::Compare {
                left: save_field("oil_gauge_backup"),
                operator: ComparisonOperator::GreaterThanOrEqual,
                right: ValueReference::Literal {
                    value: StateValue::Unsigned(0),
                },
            },
            PredicateExpression::Compare {
                left: save_field("oil_gauge_backup"),
                operator: ComparisonOperator::LessThanOrEqual,
                right: ValueReference::Literal {
                    value: StateValue::Unsigned(u16::MAX.into()),
                },
            },
        ],
    };
    let event_projection_branches = [
        (
            "",
            identity_lantern_event_projection,
            Vec::<SaveProjectionOperation>::new(),
        ),
        (
            "event-clear",
            event_projection_required,
            vec![SaveProjectionOperation::WriteRaw {
                component_id: PERSISTENT_EVENT_COMPONENT.into(),
                byte_offset: 0x1b,
                mask: vec![0x30],
                value: vec![0],
            }],
        ),
    ];
    let item_projection_branches = [
        (
            "",
            identity_lantern_item_projection,
            Vec::<SaveProjectionOperation>::new(),
        ),
        (
            "lantern-restore",
            lantern_item_projection_required,
            vec![
                SaveProjectionOperation::WriteBytesField {
                    target: ComponentFieldTarget {
                        component_id: INVENTORY_COMPONENT.into(),
                        field: "inventory".into(),
                    },
                    byte_offset: 1,
                    mask: vec![0xff],
                    value: vec![0x48],
                },
                SaveProjectionOperation::CopyValue {
                    source: ComponentFieldTarget {
                        component_id: SAVE_MENU_CONTROL_COMPONENT.into(),
                        field: "oil_gauge_backup".into(),
                    },
                    target: ComponentFieldTarget {
                        component_id: INVENTORY_COMPONENT.into(),
                        field: "oil".into(),
                    },
                },
            ],
        ),
    ];
    let saved_runtime_component_ids = vec![
        PERSISTENT_EVENT_COMPONENT.into(),
        INVENTORY_COMPONENT.into(),
        RETURN_PLACE_COMPONENT.into(),
        DUNGEON_SIX_SAVE_COMPONENT.into(),
        PLAYER_INFO_COMPONENT.into(),
        LIGHT_DROP_COMPONENT.into(),
    ];
    for index in 0_u64..3 {
        let slot = index + 1;
        for (family, use_types, success_phase) in [
            ("continue", vec![1_u64, 2], "game_continue_disp"),
            ("event", vec![3_u64, 4], "save_end"),
        ] {
            for (event_suffix, event_guard, event_operations) in &event_projection_branches {
                for (item_suffix, item_guard, item_operations) in &item_projection_branches {
                    let projection_suffix = [*event_suffix, *item_suffix]
                        .into_iter()
                        .filter(|suffix| !suffix.is_empty())
                        .collect::<Vec<_>>()
                        .join("-");
                    let id_suffix = if projection_suffix.is_empty() {
                        String::new()
                    } else {
                        format!("-{projection_suffix}")
                    };
                    let mut projection_operations = vec![
                        SaveProjectionOperation::InvalidateField {
                            target: ComponentFieldTarget {
                                component_id: PLAYER_INFO_COMPONENT.into(),
                                field: "total_time_ticks".into(),
                            },
                        },
                        SaveProjectionOperation::InvalidateField {
                            target: ComponentFieldTarget {
                                component_id: PLAYER_INFO_COMPONENT.into(),
                                field: "date_ipl_ticks".into(),
                            },
                        },
                    ];
                    projection_operations.extend(event_operations.clone());
                    projection_operations.extend(item_operations.clone());
                    file_select_branch_transitions.push(CandidateTransition {
                id: format!(
                    "transition.gz2e01.save-menu-complete-slot-{slot}-{family}{id_suffix}"
                ),
                label: format!(
                    "Complete a successful save to slot {slot} ({family} UI, {} projection)",
                    if projection_suffix.is_empty() {
                        "identity"
                    } else {
                        projection_suffix.as_str()
                    }
                ),
                scope: reset_transition.scope.clone(),
                transition_kind: TransitionKind::Other,
                approach_id: format!(
                    "save-menu.success.slot-{slot}.{family}{}",
                    if projection_suffix.is_empty() {
                        String::new()
                    } else {
                        format!(".{projection_suffix}")
                    }
                ),
                activation: ActivationContract {
                    hard_guards: PredicateExpression::All {
                        terms: vec![
                            pending_compare(
                                ValueReference::WorldExecutionActive,
                                StateValue::Boolean(true),
                            ),
                            pending_compare(
                                save_field("phase"),
                                StateValue::Text("data_save_wait2".into()),
                            ),
                            pending_compare(save_field("buffer_loaded"), StateValue::Boolean(true)),
                            pending_compare(
                                save_field("selected_index_raw"),
                                StateValue::Unsigned(index),
                            ),
                            pending_compare(
                                save_field("command_state_raw"),
                                StateValue::Unsigned(1),
                            ),
                            pending_compare(save_field("wait_timer_raw"), StateValue::Unsigned(0)),
                            PredicateExpression::Any {
                                terms: use_types
                                    .iter()
                                    .copied()
                                    .map(|use_type| {
                                        pending_compare(
                                            save_field("use_type_raw"),
                                            StateValue::Unsigned(use_type),
                                        )
                                    })
                                    .collect(),
                            },
                            event_guard.clone(),
                            item_guard.clone(),
                        ],
                    },
                    physical_obligation_ids: Vec::new(),
                    effects: vec![
                        StateOperation::SaveActiveRuntimeToSlot {
                            destination_slot: PhysicalSlotId(slot as u8),
                            destination_id_suffix: format!("save-slot-{slot}"),
                            runtime_component_ids: saved_runtime_component_ids.clone(),
                            projection_operations,
                        },
                        StateOperation::InvalidateField {
                            target: ComponentFieldTarget {
                                component_id: PLAYER_INFO_COMPONENT.into(),
                                field: "total_time_ticks".into(),
                            },
                        },
                        StateOperation::InvalidateField {
                            target: ComponentFieldTarget {
                                component_id: PLAYER_INFO_COMPONENT.into(),
                                field: "date_ipl_ticks".into(),
                            },
                        },
                        StateOperation::WriteFields {
                            component_id: RUNTIME_FILE_HEADER_COMPONENT.into(),
                            fields: BTreeMap::from([
                                ("data_num_raw".into(), StateValue::Unsigned(index)),
                                ("no_file_raw".into(), StateValue::Unsigned(0)),
                            ]),
                        },
                        StateOperation::Write {
                            target: ComponentFieldTarget {
                                component_id: SAVE_MENU_CONTROL_COMPONENT.into(),
                                field: "phase".into(),
                            },
                            value: StateValue::Text(success_phase.into()),
                        },
                    ],
                    unknown_requirements: Vec::new(),
                },
                evidence: successful_save_evidence.clone(),
            });
                }
            }
        }
    }
    file_select_branch_transitions.push(CandidateTransition {
        id: "transition.gz2e01.save-menu-write-failed".into(),
        label: "Report a failed physical save without changing any slot".into(),
        scope: reset_transition.scope.clone(),
        transition_kind: TransitionKind::Other,
        approach_id: "save-menu.failure".into(),
        activation: ActivationContract {
            hard_guards: PredicateExpression::All {
                terms: vec![
                    pending_compare(
                        ValueReference::WorldExecutionActive,
                        StateValue::Boolean(true),
                    ),
                    pending_compare(
                        save_field("phase"),
                        StateValue::Text("data_save_wait2".into()),
                    ),
                    pending_compare(save_field("command_state_raw"), StateValue::Unsigned(2)),
                    pending_compare(save_field("wait_timer_raw"), StateValue::Unsigned(0)),
                ],
            },
            physical_obligation_ids: Vec::new(),
            effects: vec![StateOperation::Write {
                target: ComponentFieldTarget {
                    component_id: SAVE_MENU_CONTROL_COMPONENT.into(),
                    field: "phase".into(),
                },
                value: StateValue::Text("memcard_command_end2".into()),
            }],
            unknown_requirements: Vec::new(),
        },
        evidence: successful_save_evidence,
    });
    let play_scene_request_evidence = play_scene_request_rule_evidence();
    let play_scene_common_guards = || {
        vec![
            name_process_guard.clone(),
            pending_compare(
                name_field("phase"),
                StateValue::Text("selection_end".into()),
            ),
        ]
    };
    let mut new_file_play_guards = play_scene_common_guards();
    new_file_play_guards.extend([
        pending_compare(
            ValueReference::ActiveRuntimeFileOrigin,
            StateValue::Text("title_file_0".into()),
        ),
        pending_compare(
            name_field("selected_entry_kind"),
            StateValue::Text("new".into()),
        ),
    ]);
    file_select_branch_transitions.push(CandidateTransition {
        id: "transition.gz2e01.file-select-new-file-request-play-scene".into(),
        label: "Request the new-file Faron Woods play scene".into(),
        scope: reset_transition.scope.clone(),
        transition_kind: TransitionKind::ActorDriven,
        approach_id: "name-scene.change-game-scene.new-file".into(),
        activation: ActivationContract {
            hard_guards: PredicateExpression::All {
                terms: new_file_play_guards,
            },
            physical_obligation_ids: Vec::new(),
            effects: vec![
                StateOperation::SetExecutionContext {
                    context: ExecutionContext::Process {
                        process_name: "PROC_NAME_SCENE".into(),
                        pending_world_load: Some(SceneLocation {
                            stage: "F_SP108".into(),
                            room: 1,
                            layer: 13,
                            spawn: 21,
                        }),
                    },
                },
                StateOperation::WriteFields {
                    component_id: NAME_SCENE_CONTROL_COMPONENT.into(),
                    fields: BTreeMap::from([
                        (
                            "phase".into(),
                            StateValue::Text("play_scene_requested".into()),
                        ),
                        (
                            "requested_process".into(),
                            StateValue::Text("PROC_PLAY_SCENE".into()),
                        ),
                    ]),
                },
            ],
            unknown_requirements: Vec::new(),
        },
        evidence: play_scene_request_evidence.clone(),
    });
    let mut existing_file_play_guards = play_scene_common_guards();
    existing_file_play_guards.extend([
        pending_compare(
            ValueReference::ActiveRuntimeFileOrigin,
            StateValue::Text("loaded_slot".into()),
        ),
        pending_compare(
            name_field("selected_entry_kind"),
            StateValue::Text("existing".into()),
        ),
    ]);
    file_select_branch_transitions.push(CandidateTransition {
        id: "transition.gz2e01.file-select-existing-file-request-play-scene".into(),
        label: "Request the loaded file's return-place play scene".into(),
        scope: reset_transition.scope.clone(),
        transition_kind: TransitionKind::ActorDriven,
        approach_id: "name-scene.change-game-scene.existing-file".into(),
        activation: ActivationContract {
            hard_guards: PredicateExpression::All {
                terms: existing_file_play_guards,
            },
            physical_obligation_ids: Vec::new(),
            effects: vec![
                StateOperation::SetPendingWorldLoadFromFields {
                    component_id: RETURN_PLACE_COMPONENT.into(),
                    stage_field: "stage".into(),
                    room_field: "room".into(),
                    spawn_field: "player_status".into(),
                    layer: -1,
                },
                StateOperation::WriteFields {
                    component_id: NAME_SCENE_CONTROL_COMPONENT.into(),
                    fields: BTreeMap::from([
                        (
                            "phase".into(),
                            StateValue::Text("play_scene_requested".into()),
                        ),
                        (
                            "requested_process".into(),
                            StateValue::Text("PROC_PLAY_SCENE".into()),
                        ),
                    ]),
                },
            ],
            unknown_requirements: Vec::new(),
        },
        evidence: play_scene_request_evidence,
    });
    let mut transitions = vec![
        name_scene_file_select_transition,
        name_scene_activation_transition,
        enter_and_initialize_transition,
        opening_process_activation_transition,
        opening_transition,
        reset_transition,
        title_key_accept_transition,
        title_request_name_scene_transition,
    ];
    transitions.extend(file_select_branch_transitions);
    transitions.sort_by(|left, right| left.id.cmp(&right.id));
    let catalog = MechanicsCatalog {
        schema: MECHANICS_CATALOG_SCHEMA.into(),
        transitions,
        obligations: Vec::new(),
        writers: Vec::new(),
        gates: Vec::new(),
        readers: Vec::new(),
        reconstruction_rules: Vec::new(),
        obstructions: Vec::new(),
        resolvers: Vec::new(),
        techniques: Vec::new(),
        microtraces: Vec::new(),
        goals: vec![Goal {
            id: GZ2E01_UNSAVED_FILE_ZERO_GOAL_ID.into(),
            label: "Remain on unsaved title-origin file 0 in an active world".into(),
            predicate: PredicateExpression::All {
                terms: vec![
                    pending_compare(
                        ValueReference::ActiveRuntimeFileOrigin,
                        StateValue::Text("title_file_0".into()),
                    ),
                    pending_compare(
                        ValueReference::WorldExecutionActive,
                        StateValue::Boolean(true),
                    ),
                ],
            },
        }],
    };
    catalog.validate()?;
    Ok(catalog)
}
