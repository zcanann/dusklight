
use super::*;
use crate::identity::{ContextSelector, ExactContext, RUNTIME_CONFIGURATION_SCHEMA};
use crate::logic::{EvidenceKind, EvidenceRecord};
use crate::orig_extraction::{MessageFlowLabel, MessageFlowSwitchStore};
use crate::state::ComponentBinding;
use crate::transition::ComponentFieldTarget;

fn scope() -> ContextScope {
    ContextScope {
        selectors: vec![ContextSelector::Exact {
            context: ExactContext {
                content_sha256: Digest([1; 32]),
                runtime_configuration_sha256: Digest([2; 32]),
            },
        }],
    }
}

fn evidence(source: Digest) -> RuleEvidence {
    RuleEvidence {
        truth: TruthStatus::Established,
        records: vec![EvidenceRecord {
            id: "evidence.message-resource".into(),
            kind: EvidenceKind::Extracted,
            source_sha256: Some(source),
            note: "Extracted from the selected language resource.".into(),
        }],
    }
}

fn program() -> MessageFlowProgram {
    let source = Digest([3; 32]);
    MessageFlowProgram {
        schema: MESSAGE_FLOW_PROGRAM_SCHEMA.into(),
        id: "message-group-3-fixture".into(),
        label: "Message group 3 fixture".into(),
        scope: scope(),
        message_group: 3,
        resource_sha256: source,
        flow_component_id: "flow.active-message".into(),
        extracted: ExtractedMessageFlow {
            header_declared_size: 100,
            resource_size: 100,
            node_count: 6,
            branch_target_count: 6,
            labels: vec![MessageFlowLabel {
                flow_id: 42,
                node_index: 0,
            }],
            nodes: vec![
                MessageFlowNode::Event {
                    index: 0,
                    event_index: 10,
                    next_target_index: 2,
                    parameter_0: 51,
                    parameter_1: 0,
                    raw_parameter_u32: 51 << 16,
                    raw_parameters: [0, 51, 0, 0],
                },
                MessageFlowNode::Branch {
                    index: 1,
                    flags: 0,
                    raw_query_index: 10,
                    query_handler_index: Some(11),
                    parameter: 11,
                    next_target_index: 0,
                },
                MessageFlowNode::Event {
                    index: 2,
                    event_index: 0,
                    next_target_index: 3,
                    parameter_0: 62,
                    parameter_1: 0,
                    raw_parameter_u32: 62 << 16,
                    raw_parameters: [0, 62, 0, 0],
                },
                MessageFlowNode::Event {
                    index: 3,
                    event_index: 14,
                    next_target_index: 4,
                    parameter_0: 0,
                    parameter_1: 12,
                    raw_parameter_u32: 12,
                    raw_parameters: [0, 0, 0, 12],
                },
                MessageFlowNode::Event {
                    index: 4,
                    event_index: 17,
                    next_target_index: 5,
                    parameter_0: 7,
                    parameter_1: 0,
                    raw_parameter_u32: 7 << 16,
                    raw_parameters: [0, 7, 0, 0],
                },
                MessageFlowNode::Unknown {
                    index: 5,
                    node_type: 9,
                    raw: [9; 8],
                },
            ],
            branch_targets: vec![2, 3, 1, u16::MAX, u16::MAX, u16::MAX],
            temporary_flag_accesses: vec![
                MessageFlowTemporaryFlagAccess {
                    node_index: 0,
                    operation: MessageFlowTemporaryFlagOperation::Set,
                    parameter_ordinal: 0,
                    label_index: 51,
                    packed_backing_coordinate: Some(0x0508),
                    friendly_name: Some("message_flow_control_f".into()),
                },
                MessageFlowTemporaryFlagAccess {
                    node_index: 1,
                    operation: MessageFlowTemporaryFlagOperation::BranchTrueWhenClear,
                    parameter_ordinal: 0,
                    label_index: 11,
                    packed_backing_coordinate: Some(0x0004),
                    friendly_name: Some("message_flow_control_a".into()),
                },
            ],
            persistent_flag_accesses: vec![MessageFlowPersistentFlagAccess {
                node_index: 2,
                operation: MessageFlowPersistentFlagOperation::Set,
                parameter_ordinal: 0,
                label_index: 62,
                packed_backing_coordinate: Some(0x0704),
                friendly_name: Some("won_gor_coron_match".into()),
            }],
            switch_accesses: vec![MessageFlowSwitchAccess {
                node_index: 3,
                operation: MessageFlowSwitchOperation::Set,
                store: MessageFlowSwitchStore::LoadedStageMemory,
                switch_index: 12,
            }],
        },
        bindings: MessageFlowBindings {
            temporary_flags: Some(MessageRawStoreBinding {
                component_kind: ComponentKind::TemporaryFlags,
                binding: ComponentBindingReference::Exact {
                    binding: ComponentBinding::Session {
                        session_id: "session.main".into(),
                    },
                },
            }),
            persistent_flags: Some(MessageRawStoreBinding {
                component_kind: ComponentKind::PersistentSave,
                binding: ComponentBindingReference::ActiveRuntimeFile,
            }),
            rupees: None,
            life: None,
            item_ownership: Vec::new(),
            switch_stores: vec![MessageSwitchStoreBinding {
                store: MessageFlowSwitchStore::LoadedStageMemory,
                component_kind: ComponentKind::StageMemory,
                binding: ComponentBindingReference::CurrentStage,
                byte_offset_base: 8,
                word_bytes: 4,
                reverse_bytes_within_word: true,
                switch_count: 128,
            }],
        },
        event_contracts: vec![MessageEventContract {
            node_index: 4,
            confirmed_operations: vec![StateOperation::Write {
                target: ComponentFieldTarget {
                    component_id: "inventory.active".into(),
                    field: "last_granted_item".into(),
                },
                value: StateValue::Unsigned(7),
            }],
            continuation: MessageEventContinuation::EncodedSuccessor,
            evidence: evidence(source),
        }],
        cleanup_edges: vec![MessageCleanupEdge {
            transition_id: "transition.cleanup.central-message".into(),
            label: "Central message completion cleanup".into(),
            approach_id: "approach.cleanup.central-message".into(),
            activation: PredicateExpression::Compare {
                left: ValueReference::FlowNode {
                    flow_component_id: "flow.active-message".into(),
                },
                operator: ComparisonOperator::Equal,
                right: ValueReference::Literal {
                    value: StateValue::Text("message-cleanup-ready".into()),
                },
            },
            packed_backing_coordinates: vec![0x0001, 0x0002, 0x0004],
            evidence: evidence(source),
        }],
        evidence: evidence(source),
    }
}

fn import_profile() -> MessageFlowImportProfile {
    let template = program();
    MessageFlowImportProfile {
        schema: MESSAGE_FLOW_IMPORT_PROFILE_SCHEMA.into(),
        id: "gcn-us-fixture".into(),
        content_sha256: Digest([1; 32]),
        language_bundles: BTreeMap::from([("en".into(), "us".into())]),
        flow_component_id: template.flow_component_id,
        bindings: template.bindings,
        evidence: evidence(Digest([9; 32])),
    }
}

fn runtime_configuration() -> RuntimeConfiguration {
    RuntimeConfiguration {
        schema: RUNTIME_CONFIGURATION_SCHEMA.into(),
        content_sha256: Digest([1; 32]),
        language: "en".into(),
        settings: BTreeMap::new(),
    }
}

fn extracted_archive(group: u16, source: u8) -> ExtractedOrigMessageArchive {
    ExtractedOrigMessageArchive {
        relative_path: format!("files/res/Msgus/bmgres{group}.arc"),
        archive_sha256: Digest([source.wrapping_add(1); 32]),
        locale_bundle: "us".into(),
        message_group: group,
        resource_name: format!("zel_{group:02}.bmg"),
        resource_sha256: Digest([source; 32]),
        flow: program().extracted,
    }
}

#[test]
fn compiles_known_handlers_branches_cleanup_and_event_handoffs() {
    let program = program();
    program.validate().unwrap();
    let compiled = program.compile().unwrap();
    assert_eq!(compiled.entry_points[0].flow_id, 42);
    assert_eq!(compiled.unresolved_nodes.len(), 1);
    assert_eq!(compiled.mechanics.transitions.len(), 7);
    assert_eq!(compiled.mechanics.readers.len(), 2);
    assert_eq!(compiled.aliases.len(), 3);

    let set_temp = compiled
        .mechanics
        .transitions
        .iter()
        .find(|transition| transition.label.contains("event 10"))
        .unwrap();
    assert!(matches!(
        &set_temp.activation.effects[0],
        StateOperation::WriteBoundRaw {
            component_kind: ComponentKind::TemporaryFlags,
            byte_offset: 5,
            mask,
            value,
            ..
        } if mask == &[0x08] && value == &[0x08]
    ));

    let branch_clear = compiled
        .mechanics
        .transitions
        .iter()
        .find(|transition| transition.label == "Take message branch 1 at node 1")
        .unwrap();
    assert!(matches!(
        &branch_clear.activation.hard_guards,
        PredicateExpression::All { terms }
            if matches!(
                &terms[1],
                PredicateExpression::Compare {
                    right: ValueReference::Literal {
                        value: StateValue::Unsigned(0)
                    },
                    ..
                }
            )
    ));
    assert!(matches!(
        &branch_clear.activation.effects[0],
        StateOperation::BranchFlow {
            destination_node_id,
            ..
        } if destination_node_id.ends_with(".3")
    ));

    let switch = compiled
        .mechanics
        .transitions
        .iter()
        .find(|transition| transition.label.contains("event 14"))
        .unwrap();
    assert!(matches!(
        &switch.activation.effects[0],
        StateOperation::WriteBoundRaw {
            component_kind: ComponentKind::StageMemory,
            binding: ComponentBindingReference::CurrentStage,
            byte_offset: 10,
            mask,
            value,
        } if mask == &[0x10] && value == &[0x10]
    ));

    let cleanup = compiled
        .mechanics
        .transitions
        .iter()
        .find(|transition| transition.id == "transition.cleanup.central-message")
        .unwrap();
    assert_eq!(cleanup.activation.effects.len(), 3);
    assert!(cleanup.activation.effects.iter().all(|operation| matches!(
        operation,
        StateOperation::WriteBoundRaw { value, .. } if value == &[0]
    )));
    let event_handoff = compiled
        .mechanics
        .transitions
        .iter()
        .find(|transition| transition.label.contains("event 17"))
        .unwrap();
    assert!(event_handoff.activation.unknown_requirements.is_empty());
    assert!(matches!(
        event_handoff.activation.effects.as_slice(),
        [
            StateOperation::Write { .. },
            StateOperation::AdvanceFlow { .. }
        ]
    ));
    assert_eq!(
        MessageFlowProgram::decode_canonical(&program.canonical_bytes().unwrap()).unwrap(),
        program
    );
    assert_eq!(
        CompiledMessageFlowProgram::decode_canonical(&compiled.canonical_bytes().unwrap()).unwrap(),
        compiled
    );
}

#[test]
fn compiles_nonzero_rupee_query_without_guessing_wallet_capacity() {
    let mut program = program();
    let MessageFlowNode::Branch {
        raw_query_index,
        query_handler_index,
        parameter,
        ..
    } = &mut program.extracted.nodes[1]
    else {
        panic!("fixture node 1 must be a branch");
    };
    *raw_query_index = 6;
    *query_handler_index = Some(4);
    *parameter = 300;
    program
        .extracted
        .temporary_flag_accesses
        .retain(|access| access.node_index != 1);
    program.bindings.rupees = Some(ComponentFieldTarget {
        component_id: "inventory-and-resources".into(),
        field: "rupees".into(),
    });

    let compiled = program.compile().unwrap();
    for (outcome, operator) in [
        (0, ComparisonOperator::GreaterThanOrEqual),
        (1, ComparisonOperator::LessThan),
    ] {
        let transition = compiled
            .mechanics
            .transitions
            .iter()
            .find(|transition| {
                transition.label == format!("Take message branch {outcome} at node 1")
            })
            .unwrap();
        let transition_id = &transition.id;
        assert!(transition.activation.unknown_requirements.is_empty());
        assert!(matches!(
            &transition.activation.hard_guards,
            PredicateExpression::All { terms }
                if matches!(
                    &terms[1],
                    PredicateExpression::Compare {
                        left: ValueReference::ComponentField {
                            component_id,
                            field,
                        },
                        operator: actual_operator,
                        right: ValueReference::Literal {
                            value: StateValue::Unsigned(300),
                        },
                    } if component_id == "inventory-and-resources"
                        && field == "rupees"
                        && *actual_operator == operator
                )
        ));
        let reader = compiled
            .mechanics
            .readers
            .iter()
            .find(|reader| reader.consuming_transition_id == *transition_id)
            .unwrap();
        assert!(matches!(
            &reader.source,
            ValueReference::ComponentField {
                component_id,
                field,
            } if component_id == "inventory-and-resources" && field == "rupees"
        ));
    }

    let MessageFlowNode::Event {
        event_index,
        parameter_0,
        parameter_1,
        raw_parameter_u32,
        raw_parameters,
        ..
    } = &mut program.extracted.nodes[2]
    else {
        panic!("fixture node 2 must be an event");
    };
    *event_index = 3;
    *parameter_0 = 0;
    *parameter_1 = 300;
    *raw_parameter_u32 = 300;
    *raw_parameters = 300_u32.to_be_bytes();
    program
        .extracted
        .persistent_flag_accesses
        .retain(|access| access.node_index != 2);
    let compiled = program.compile().unwrap();
    let debit = compiled
        .mechanics
        .transitions
        .iter()
        .find(|transition| transition.label == "Execute message event 3 at node 2")
        .unwrap();
    assert!(debit.activation.unknown_requirements.is_empty());
    assert!(matches!(
        debit.activation.effects.as_slice(),
        [
            StateOperation::DebitUnsigned {
                target: ComponentFieldTarget {
                    component_id,
                    field,
                },
                amount: 300,
            },
            StateOperation::AdvanceFlow { .. },
        ] if component_id == "inventory-and-resources" && field == "rupees"
    ));

    let MessageFlowNode::Branch { parameter, .. } = &mut program.extracted.nodes[1] else {
        unreachable!()
    };
    *parameter = 0;
    let compiled = program.compile().unwrap();
    assert!(
        compiled
            .mechanics
            .transitions
            .iter()
            .filter(|transition| transition.label.contains("at node 1"))
            .all(|transition| transition.activation.unknown_requirements.len() == 1)
    );
}

#[test]
fn compiles_life_threshold_query_and_saturating_damage_event() {
    let mut program = program();
    let MessageFlowNode::Branch {
        raw_query_index,
        query_handler_index,
        parameter,
        ..
    } = &mut program.extracted.nodes[1]
    else {
        panic!("fixture node 1 must be a branch");
    };
    *raw_query_index = 31;
    *query_handler_index = Some(32);
    *parameter = 12;
    program
        .extracted
        .temporary_flag_accesses
        .retain(|access| access.node_index != 1);
    program.bindings.life = Some(ComponentFieldTarget {
        component_id: "inventory-and-resources".into(),
        field: "life".into(),
    });

    let MessageFlowNode::Event {
        event_index,
        parameter_0,
        parameter_1,
        raw_parameter_u32,
        raw_parameters,
        ..
    } = &mut program.extracted.nodes[2]
    else {
        panic!("fixture node 2 must be an event");
    };
    *event_index = 5;
    *parameter_0 = 0;
    *parameter_1 = 4;
    *raw_parameter_u32 = 4;
    *raw_parameters = 4_u32.to_be_bytes();
    program
        .extracted
        .persistent_flag_accesses
        .retain(|access| access.node_index != 2);

    let compiled = program.compile().unwrap();
    for (outcome, operator) in [
        (0, ComparisonOperator::GreaterThanOrEqual),
        (1, ComparisonOperator::LessThan),
    ] {
        let transition = compiled
            .mechanics
            .transitions
            .iter()
            .find(|transition| {
                transition.label == format!("Take message branch {outcome} at node 1")
            })
            .unwrap();
        let transition_id = &transition.id;
        assert!(transition.activation.unknown_requirements.is_empty());
        assert!(matches!(
            &transition.activation.hard_guards,
            PredicateExpression::All { terms }
                if matches!(
                    &terms[1],
                    PredicateExpression::Compare {
                        left: ValueReference::ComponentField {
                            component_id,
                            field,
                        },
                        operator: actual_operator,
                        right: ValueReference::Literal {
                            value: StateValue::Unsigned(12),
                        },
                    } if component_id == "inventory-and-resources"
                        && field == "life"
                        && *actual_operator == operator
                )
        ));
        let reader = compiled
            .mechanics
            .readers
            .iter()
            .find(|reader| reader.consuming_transition_id == *transition_id)
            .unwrap();
        assert!(matches!(
            &reader.source,
            ValueReference::ComponentField {
                component_id,
                field,
            } if component_id == "inventory-and-resources" && field == "life"
        ));
    }

    let damage = compiled
        .mechanics
        .transitions
        .iter()
        .find(|transition| transition.label == "Execute message event 5 at node 2")
        .unwrap();
    assert!(damage.activation.unknown_requirements.is_empty());
    assert!(matches!(
        damage.activation.effects.as_slice(),
        [
            StateOperation::DebitUnsigned {
                target: ComponentFieldTarget {
                    component_id,
                    field,
                },
                amount: 4,
            },
            StateOperation::AdvanceFlow { .. },
        ] if component_id == "inventory-and-resources" && field == "life"
    ));
}

#[test]
fn unsupported_handlers_stay_unknown_and_unknown_nodes_have_no_edge() {
    let mut program = program();
    program.event_contracts.clear();
    let compiled = program.compile().unwrap();
    let unsupported = compiled
        .mechanics
        .transitions
        .iter()
        .find(|transition| transition.label.contains("event 17"))
        .unwrap();
    assert_eq!(unsupported.activation.unknown_requirements.len(), 1);
    assert!(
        !compiled
            .mechanics
            .transitions
            .iter()
            .any(|transition| transition.label.contains("node 5"))
    );
    assert_eq!(compiled.unresolved_nodes[0].node_index, 5);
}

#[test]
fn exact_flow_jumps_override_encoded_successors_and_dynamic_jumps_stay_unknown() {
    let mut program = program();
    program.extracted.node_count = 12;
    program.extracted.branch_target_count = 10;
    program
        .extracted
        .branch_targets
        .extend([u16::MAX, 9, 10, 11]);
    program.extracted.labels.push(MessageFlowLabel {
        flow_id: 99,
        node_index: 7,
    });
    program.extracted.nodes.extend([
        MessageFlowNode::Event {
            index: 6,
            event_index: 9,
            next_target_index: 6,
            parameter_0: 0,
            parameter_1: 99,
            raw_parameter_u32: 99,
            raw_parameters: [0, 0, 0, 99],
        },
        MessageFlowNode::Message {
            index: 7,
            flags: 0,
            message_index: 4,
            next_node_index: u16::MAX,
            unknown: 0,
        },
        MessageFlowNode::Event {
            index: 8,
            event_index: 12,
            next_target_index: 7,
            parameter_0: 0,
            parameter_1: 0,
            raw_parameter_u32: 0,
            raw_parameters: [0; 4],
        },
        MessageFlowNode::Event {
            index: 9,
            event_index: 19,
            next_target_index: 8,
            parameter_0: 0,
            parameter_1: 0,
            raw_parameter_u32: 0,
            raw_parameters: [0; 4],
        },
        MessageFlowNode::Event {
            index: 10,
            event_index: 42,
            next_target_index: 9,
            parameter_0: 0,
            parameter_1: 0,
            raw_parameter_u32: 0,
            raw_parameters: [0; 4],
        },
        MessageFlowNode::Message {
            index: 11,
            flags: 0,
            message_index: 5,
            next_node_index: u16::MAX,
            unknown: 0,
        },
    ]);

    let compiled = program.compile().unwrap();
    let jump = compiled
        .mechanics
        .transitions
        .iter()
        .find(|transition| transition.label.contains("event 9 at node 6"))
        .unwrap();
    assert!(jump.activation.unknown_requirements.is_empty());
    assert!(matches!(
        jump.activation.effects.as_slice(),
        [StateOperation::AdvanceFlow { node_id, .. }] if node_id.ends_with(".7")
    ));
    for (event_index, destination) in [(12_u8, 9_u16), (19, 10), (42, 11)] {
        let transition = compiled
            .mechanics
            .transitions
            .iter()
            .find(|transition| transition.label.contains(&format!("event {event_index} ")))
            .unwrap();
        assert!(transition.activation.unknown_requirements.is_empty());
        assert!(matches!(
            transition.activation.effects.as_slice(),
            [StateOperation::AdvanceFlow { node_id, .. }]
                if node_id.ends_with(&format!(".{destination}"))
        ));
    }

    let MessageFlowNode::Event {
        raw_parameter_u32,
        parameter_1,
        raw_parameters,
        ..
    } = &mut program.extracted.nodes[6]
    else {
        unreachable!()
    };
    *raw_parameter_u32 = 0;
    *parameter_1 = 0;
    *raw_parameters = [0; 4];
    let dynamic = program.compile().unwrap();
    let jump = dynamic
        .mechanics
        .transitions
        .iter()
        .find(|transition| transition.label.contains("event 9 at node 6"))
        .unwrap();
    assert_eq!(jump.activation.unknown_requirements.len(), 1);
    assert!(jump.activation.effects.is_empty());
}

#[test]
fn event_request_publishes_ids_without_granting_the_item() {
    let mut program = program();
    program.extracted.node_count = 7;
    program.extracted.branch_target_count = 7;
    program.extracted.branch_targets.push(u16::MAX);
    program.extracted.nodes.push(MessageFlowNode::Event {
        index: 6,
        event_index: 8,
        next_target_index: 6,
        parameter_0: 1,
        parameter_1: 0xa3,
        raw_parameter_u32: 0x0001_00a3,
        raw_parameters: [0, 1, 0, 0xa3],
    });

    let compiled = program.compile().unwrap();
    let request = compiled
        .mechanics
        .transitions
        .iter()
        .find(|transition| transition.label.contains("event 8 at node 6"))
        .unwrap();
    assert!(request.activation.unknown_requirements.is_empty());
    assert!(matches!(
        request.activation.effects.as_slice(),
        [
            StateOperation::Write {
                target: ComponentFieldTarget { field: event_id, .. },
                value: StateValue::Unsigned(1),
            },
            StateOperation::Write {
                target: ComponentFieldTarget { field: item_id, .. },
                value: StateValue::Unsigned(0xa3),
            },
            StateOperation::AdvanceFlow { node_id, .. },
        ] if event_id == "event_id" && item_id == "item_id" && node_id.ends_with(".end")
    ));
    assert!(!request.activation.effects.iter().any(|operation| matches!(
        operation,
        StateOperation::SetBitFromValue { .. }
            | StateOperation::WriteRaw { .. }
            | StateOperation::WriteBoundRaw { .. }
    )));

    let MessageFlowNode::Event {
        parameter_0,
        parameter_1,
        raw_parameter_u32,
        raw_parameters,
        ..
    } = &mut program.extracted.nodes[6]
    else {
        unreachable!()
    };
    *parameter_0 = 27;
    *parameter_1 = 0;
    *raw_parameter_u32 = 27 << 16;
    *raw_parameters = [0, 27, 0, 0];
    let fundraising = program.compile().unwrap();
    let request = fundraising
        .mechanics
        .transitions
        .iter()
        .find(|transition| transition.label.contains("event 8 at node 6"))
        .unwrap();
    assert_eq!(request.activation.unknown_requirements.len(), 1);
    assert_eq!(request.activation.effects.len(), 3);
}

#[test]
fn item_query_and_event_share_the_same_bound_ownership_bit() {
    let mut program = program();
    program.event_contracts.clear();
    program.bindings.item_ownership = vec![MessageItemOwnershipBinding {
        item_id: 0xa3,
        label: "Lanayru Vessel owned".into(),
        component_kind: ComponentKind::Custom {
            id: "player-light-drop".into(),
        },
        binding: ComponentBindingReference::ActiveRuntimeFile,
        byte_offset: 4,
        mask: 0x04,
    }];
    let MessageFlowNode::Event {
        parameter_0,
        raw_parameter_u32,
        raw_parameters,
        ..
    } = &mut program.extracted.nodes[4]
    else {
        unreachable!()
    };
    *parameter_0 = 0xa3;
    *raw_parameter_u32 = 0xa3 << 16;
    *raw_parameters = [0, 0xa3, 0, 0];
    program.extracted.nodes[5] = MessageFlowNode::Branch {
        index: 5,
        flags: 0,
        raw_query_index: 21,
        query_handler_index: Some(22),
        parameter: 0xa3,
        next_target_index: 4,
    };

    let compiled = program.compile().unwrap();
    let item_set = compiled
        .mechanics
        .transitions
        .iter()
        .find(|transition| transition.label.contains("event 17 at node 4"))
        .unwrap();
    assert!(item_set.activation.unknown_requirements.is_empty());
    assert!(matches!(
        item_set.activation.effects.first(),
        Some(StateOperation::WriteBoundRaw {
            component_kind: ComponentKind::Custom { id },
            binding: ComponentBindingReference::ActiveRuntimeFile,
            byte_offset: 4,
            mask,
            value,
        }) if id == "player-light-drop" && mask == &[0x04] && value == &[0x04]
    ));

    let branches = compiled
        .mechanics
        .transitions
        .iter()
        .filter(|transition| {
            transition.label.contains("branch") && transition.label.contains("node 5")
        })
        .collect::<Vec<_>>();
    assert_eq!(branches.len(), 2);
    assert!(
        branches
            .iter()
            .all(|transition| transition.activation.unknown_requirements.is_empty())
    );
    assert_eq!(
        compiled
            .mechanics
            .readers
            .iter()
            .filter(|reader| reader.consuming_transition_id.contains("node-5"))
            .count(),
        2
    );
    assert!(compiled.aliases.iter().any(|alias| {
        alias.label == "Lanayru Vessel owned"
            && alias.raw.byte_offset == 4
            && alias.raw.mask == [0x04]
    }));
}

#[test]
fn constructs_every_selected_resource_with_exact_scope_and_profile_bindings() {
    let profile = import_profile();
    let runtime = runtime_configuration();
    let group_three = extracted_archive(3, 3);
    let group_zero = extracted_archive(0, 4);
    let programs = construct_selected_message_flow_programs(
        profile.content_sha256,
        &runtime,
        &profile,
        "us",
        &[&group_three, &group_zero],
    )
    .unwrap();

    assert_eq!(
        programs
            .iter()
            .map(|program| program.message_group)
            .collect::<Vec<_>>(),
        vec![0, 3]
    );
    assert!(programs.iter().all(|program| {
        program.bindings == profile.bindings
            && program.event_contracts.is_empty()
            && program.cleanup_edges.is_empty()
            && program.scope.selectors
                == vec![ContextSelector::Exact {
                    context: runtime.exact_context().unwrap(),
                }]
            && program.evidence.records.iter().any(|record| {
                record.kind == EvidenceKind::Extracted
                    && record.source_sha256 == Some(program.resource_sha256)
            })
    }));
    assert!(programs.iter().all(|program| program.compile().is_ok()));
    let set = MessageFlowProgramSet {
        schema: MESSAGE_FLOW_PROGRAM_SET_SCHEMA.into(),
        profile_sha256: profile.digest().unwrap(),
        bundle_sha256: Digest([8; 32]),
        exact_context: runtime.exact_context().unwrap(),
        locale_bundle: "us".into(),
        programs,
    };
    assert_eq!(
        MessageFlowProgramSet::decode_canonical(&set.canonical_bytes().unwrap()).unwrap(),
        set
    );
    assert_eq!(
        MessageFlowImportProfile::decode_canonical(&profile.canonical_bytes().unwrap()).unwrap(),
        profile
    );

    let mut long_id_profile = profile.clone();
    long_id_profile.id = "a".repeat(128);
    assert!(
        construct_selected_message_flow_programs(
            long_id_profile.content_sha256,
            &runtime,
            &long_id_profile,
            "us",
            &[&group_zero],
        )
        .is_ok()
    );
}

#[test]
fn bundled_gz2e01_profile_maps_only_source_audited_backings() {
    let profile = bundled_gz2e01_english_message_flow_profile().unwrap();
    assert_eq!(profile.id, "gcn-us-1.0-gz2e01-en");
    assert_eq!(profile.flow_component_id, "message-session");
    assert_eq!(profile.language_bundles.len(), 1);
    assert_eq!(
        profile.language_bundles.get("en").map(String::as_str),
        Some("us")
    );
    assert!(!profile.language_bundles.contains_key("fr"));
    assert!(profile.bindings.temporary_flags.is_some());
    assert_eq!(
        profile.bindings.persistent_flags,
        Some(MessageRawStoreBinding {
            component_kind: ComponentKind::Custom {
                id: "persistent-event-registers".into(),
            },
            binding: ComponentBindingReference::ActiveRuntimeFile,
        })
    );
    assert_eq!(
        profile.bindings.rupees,
        Some(ComponentFieldTarget {
            component_id: "inventory-and-resources".into(),
            field: "rupees".into(),
        })
    );
    assert_eq!(
        profile.bindings.life,
        Some(ComponentFieldTarget {
            component_id: "inventory-and-resources".into(),
            field: "life".into(),
        })
    );
    assert_eq!(profile.bindings.item_ownership.len(), 1);
    assert_eq!(profile.bindings.item_ownership[0].item_id, 0xa3);
    assert_eq!(profile.bindings.item_ownership[0].byte_offset, 4);
    assert_eq!(profile.bindings.item_ownership[0].mask, 0x04);
    assert_eq!(profile.bindings.switch_stores.len(), 1);
    assert_eq!(
        profile.bindings.switch_stores[0].store,
        MessageFlowSwitchStore::LoadedStageMemory
    );
    assert_eq!(profile.bindings.switch_stores[0].byte_offset_base, 0x08);
    assert_eq!(profile.bindings.switch_stores[0].switch_count, 128);
    assert!(
        profile
            .evidence
            .records
            .iter()
            .all(|record| record.kind == EvidenceKind::SourceAudited)
    );
}

#[test]
fn bundled_gz2p01_profile_selects_all_pal_resources_without_semantic_backings() {
    let profile = bundled_gz2p01_structural_message_flow_profile().unwrap();
    assert_eq!(profile.id, "gcn-pal-1.0-gz2p01-structural");
    assert_eq!(
        profile.content_sha256.to_string(),
        "b1a8934598abc52dba2b23241664dd50521f45c9c6b6b18d5aaf0bc7a99d8170"
    );
    assert_eq!(
        profile.language_bundles,
        BTreeMap::from([
            ("de".into(), "de".into()),
            ("en".into(), "uk".into()),
            ("es".into(), "sp".into()),
            ("fr".into(), "fr".into()),
            ("it".into(), "it".into()),
        ])
    );
    assert!(profile.bindings.temporary_flags.is_none());
    assert!(profile.bindings.persistent_flags.is_none());
    assert!(profile.bindings.rupees.is_none());
    assert!(profile.bindings.life.is_none());
    assert!(profile.bindings.item_ownership.is_empty());
    assert!(profile.bindings.switch_stores.is_empty());
}

#[test]
fn selected_resource_construction_rejects_ambiguity_and_keeps_unmapped_stores_unknown() {
    let profile = import_profile();
    let runtime = runtime_configuration();
    let first = extracted_archive(3, 3);
    let duplicate = extracted_archive(3, 4);
    assert_eq!(
        construct_selected_message_flow_programs(
            profile.content_sha256,
            &runtime,
            &profile,
            "us",
            &[&first, &duplicate],
        )
        .unwrap_err()
        .field(),
        "message_flow_import_profile.selected_resources"
    );

    let mut missing_store = profile;
    missing_store.bindings.temporary_flags = None;
    missing_store.bindings.persistent_flags = None;
    missing_store.bindings.switch_stores.clear();
    let programs = construct_selected_message_flow_programs(
        missing_store.content_sha256,
        &runtime,
        &missing_store,
        "us",
        &[&first],
    )
    .unwrap();
    let compiled = programs[0].compile().unwrap();
    let switch_event = compiled
        .mechanics
        .transitions
        .iter()
        .find(|transition| transition.label.contains("event 14"))
        .unwrap();
    assert!(
        switch_event
            .activation
            .effects
            .iter()
            .all(|effect| { !matches!(effect, StateOperation::WriteBoundRaw { .. }) })
    );
    assert!(
        switch_event
            .activation
            .unknown_requirements
            .iter()
            .any(|requirement| requirement.id.contains("switch-backing"))
    );

    let temporary_event = compiled
        .mechanics
        .transitions
        .iter()
        .find(|transition| transition.label.contains("event 10"))
        .unwrap();
    assert!(
        temporary_event
            .activation
            .effects
            .iter()
            .all(|effect| { !matches!(effect, StateOperation::WriteBoundRaw { .. }) })
    );
    assert!(
        temporary_event
            .activation
            .unknown_requirements
            .iter()
            .any(|requirement| requirement.id.contains("temporary-parameter-0-backing"))
    );

    let persistent_event = compiled
        .mechanics
        .transitions
        .iter()
        .find(|transition| transition.label.contains("event 0"))
        .unwrap();
    assert!(
        persistent_event
            .activation
            .effects
            .iter()
            .all(|effect| { !matches!(effect, StateOperation::WriteBoundRaw { .. }) })
    );
    assert!(
        persistent_event
            .activation
            .unknown_requirements
            .iter()
            .any(|requirement| requirement.id.contains("persistent-parameter-0-backing"))
    );
}

#[test]
fn rejects_bad_targets_and_inexact_control_contracts() {
    let mut bad_target = program();
    let MessageFlowNode::Event {
        next_target_index, ..
    } = &mut bad_target.extracted.nodes[0]
    else {
        unreachable!()
    };
    *next_target_index = 99;
    assert_eq!(
        bad_target.validate().unwrap_err().field(),
        "message_flow_program.extracted.event"
    );

    let mut bad_contract = program();
    bad_contract.event_contracts[0].continuation = MessageEventContinuation::ContractControlled;
    assert_eq!(
        bad_contract.validate().unwrap_err().field(),
        "message_flow_program.event_contracts.continuation"
    );

    let mut mismatched_access = program();
    mismatched_access.extracted.temporary_flag_accesses[0].label_index = 52;
    assert_eq!(
        mismatched_access.validate().unwrap_err().field(),
        "message_flow_program.extracted.temporary_flag_accesses"
    );

    let mut unconditional_cleanup = program();
    unconditional_cleanup.cleanup_edges[0].activation = PredicateExpression::True;
    assert_eq!(
        unconditional_cleanup.validate().unwrap_err().field(),
        "message_flow_program.cleanup.activation"
    );
}
