
use super::*;
use crate::identity::{
    ContentFingerprint, ContentIdentity, ContextSelector, GamePlatform, GameRegion,
};
use crate::logic::{
    ComparisonOperator, EvidenceKind, EvidenceRecord, PredicateExpression, RuleEvidence,
    ValueReference,
};
use crate::message_flow::{MESSAGE_FLOW_PROGRAM_SCHEMA, MessageFlowBindings};
use crate::orig_discovery::{
    EXTRACTED_ORIG_BUNDLE_SCHEMA, ExtractedOrigBundle, ExtractedOrigStageArchive,
    ORIG_INPUT_SCAN_SCHEMA, OrigFileRecord, OrigInputScan,
};
use crate::orig_extraction::{
    ExtractedActorPlacement, ExtractedMessageFlow, ExtractedStageData, ExtractedStageInformation,
    MessageFlowLabel, MessageFlowNode,
};
use crate::state::{ComponentBindingReference, ComponentKind, StateValue};
use crate::transition::{ComponentFieldTarget, UnknownRequirement};
use serde::Serialize;

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

fn evidence() -> RuleEvidence {
    RuleEvidence {
        truth: TruthStatus::Established,
        records: vec![EvidenceRecord {
            id: "evidence.fixture".into(),
            kind: EvidenceKind::SourceAudited,
            source_sha256: Some(Digest([8; 32])),
            note: "Source-audited fixture.".into(),
        }],
    }
}

fn resource(group: u8) -> CompiledMessageFlowResource {
    let resource_sha256 = Digest([group + 20; 32]);
    let source_program = MessageFlowProgram {
        schema: MESSAGE_FLOW_PROGRAM_SCHEMA.into(),
        id: format!("message-program.fixture.group-{group}"),
        label: format!("Message group {group}"),
        scope: scope(),
        message_group: group,
        resource_sha256,
        flow_component_id: "flow.active-message".into(),
        extracted: ExtractedMessageFlow {
            header_declared_size: 64,
            resource_size: 64,
            node_count: 1,
            branch_target_count: 0,
            labels: vec![MessageFlowLabel {
                flow_id: 7,
                node_index: 0,
            }],
            nodes: vec![MessageFlowNode::Message {
                index: 0,
                flags: 0,
                message_index: 3,
                next_node_index: u16::MAX,
                unknown: 0,
            }],
            branch_targets: Vec::new(),
            temporary_flag_accesses: Vec::new(),
            persistent_flag_accesses: Vec::new(),
            switch_accesses: Vec::new(),
        },
        bindings: MessageFlowBindings {
            temporary_flags: None,
            persistent_flags: None,
            rupees: None,
            life: None,
            item_ownership: vec![MessageItemOwnershipBinding {
                item_id: 7,
                label: "Fixture item owned".into(),
                component_kind: ComponentKind::Inventory,
                binding: ComponentBindingReference::ActiveRuntimeFile,
                byte_offset: 0,
                mask: 0x80,
            }],
            switch_stores: Vec::new(),
        },
        event_contracts: Vec::new(),
        cleanup_edges: Vec::new(),
        evidence: RuleEvidence {
            truth: TruthStatus::Established,
            records: vec![EvidenceRecord {
                id: format!("evidence.message.group-{group}"),
                kind: EvidenceKind::Extracted,
                source_sha256: Some(resource_sha256),
                note: format!("Extracted message group {group}."),
            }],
        },
    };
    let compiled_program = source_program.compile().unwrap();
    CompiledMessageFlowResource {
        message_group: group,
        archive_sha256: Digest([group + 10; 32]),
        resource_sha256,
        source_program,
        compiled_program,
    }
}

fn compiled_set() -> CompiledMessageFlowSet {
    let resources = vec![resource(0), resource(3)];
    let (facts, mechanics) = merged_catalogs(&resources).unwrap();
    CompiledMessageFlowSet {
        schema: COMPILED_MESSAGE_FLOW_SET_SCHEMA.into(),
        program_set_sha256: Digest([4; 32]),
        overlay_set_sha256: None,
        exact_context: scope()
            .selectors
            .into_iter()
            .next()
            .map(|selector| {
                let ContextSelector::Exact { context } = selector else {
                    unreachable!()
                };
                context
            })
            .unwrap(),
        locale_bundle: "us".into(),
        resources,
        facts,
        mechanics,
    }
}

fn compiled_set_for_content(content_sha256: Digest) -> CompiledMessageFlowSet {
    let mut set = compiled_set();
    set.exact_context.content_sha256 = content_sha256;
    let scope = ContextScope {
        selectors: vec![ContextSelector::Exact {
            context: set.exact_context.clone(),
        }],
    };
    for resource in &mut set.resources {
        resource.source_program.scope = scope.clone();
        resource.compiled_program = resource.source_program.compile().unwrap();
    }
    (set.facts, set.mechanics) = merged_catalogs(&set.resources).unwrap();
    set.validate().unwrap();
    set
}

fn entry_bundle() -> ExtractedOrigBundle {
    #[derive(Serialize)]
    struct FileManifest<'a> {
        schema: &'static str,
        product_id: &'a str,
        files: &'a [OrigFileRecord],
    }

    let actor_path = "files/res/Stage/F_SP103/Room0.arc";
    let stage_path = "files/res/Stage/F_SP103/Stage.arc";
    let files = vec![
        OrigFileRecord {
            relative_path: actor_path.into(),
            bytes: 100,
            sha256: Digest([30; 32]),
        },
        OrigFileRecord {
            relative_path: stage_path.into(),
            bytes: 120,
            sha256: Digest([33; 32]),
        },
        OrigFileRecord {
            relative_path: "sys/main.dol".into(),
            bytes: 200,
            sha256: Digest([32; 32]),
        },
    ];
    let manifest_digest = |records: &[OrigFileRecord]| {
        Digest(
            Sha256::digest(
                canonical_json(&FileManifest {
                    schema: "dusklight.route-planner.orig-file-manifest/v1",
                    product_id: "GZ2E01",
                    files: records,
                })
                .unwrap(),
            )
            .into(),
        )
    };
    let game_data_sha256 = manifest_digest(&files);
    let resource_manifest_sha256 = manifest_digest(&files[..2]);
    let fingerprint = ContentFingerprint {
        platform: GamePlatform::GameCube,
        region: GameRegion::Usa,
        revision: "fixture".into(),
        product_id: "GZ2E01".into(),
        executable_sha256: Digest([32; 32]),
        game_data_sha256,
        resource_manifest_sha256,
    };
    ExtractedOrigBundle {
        schema: EXTRACTED_ORIG_BUNDLE_SCHEMA.into(),
        content: ContentIdentity::new("fixture-gz2e01", fingerprint.clone()).unwrap(),
        input_scan: OrigInputScan {
            schema: ORIG_INPUT_SCAN_SCHEMA.into(),
            fingerprint,
            file_manifest_sha256: game_data_sha256,
            files,
            extractable_archive_paths: vec![actor_path.into(), stage_path.into()],
        },
        stages: vec![
            ExtractedOrigStageArchive {
                relative_path: actor_path.into(),
                archive_sha256: Digest([30; 32]),
                resource_name: "room.dzr".into(),
                resource_sha256: Digest([31; 32]),
                stage: ExtractedStageData {
                    chunks: Vec::new(),
                    stage_information: None,
                    room_transforms: Vec::new(),
                    file_lists: Vec::new(),
                    room_read_table: Vec::new(),
                    cameras: Vec::new(),
                    camera_arrows: Vec::new(),
                    paths: Vec::new(),
                    path_points: Vec::new(),
                    scene_transitions: Vec::new(),
                    map_events: Vec::new(),
                    demo_archive_banks: Vec::new(),
                    actor_placements: vec![ExtractedActorPlacement {
                        chunk_tag: "ACTR".into(),
                        record_index: 4,
                        layer: Some(2),
                        name: "Npc_Gro".into(),
                        parameters: 0,
                        position: [1.0, 2.0, 3.0],
                        angle: [0; 3],
                        set_id: 0xff,
                        scale_raw: None,
                        raw_hex: "0011223344556677".into(),
                    }],
                    treasure_placements: Vec::new(),
                    player_spawns: Vec::new(),
                },
            },
            ExtractedOrigStageArchive {
                relative_path: stage_path.into(),
                archive_sha256: Digest([33; 32]),
                resource_name: "stage.dzs".into(),
                resource_sha256: Digest([34; 32]),
                stage: ExtractedStageData {
                    chunks: Vec::new(),
                    stage_information: Some(ExtractedStageInformation {
                        message_group: 3,
                        raw_hex: "00000003".into(),
                    }),
                    room_transforms: Vec::new(),
                    file_lists: Vec::new(),
                    room_read_table: Vec::new(),
                    cameras: Vec::new(),
                    camera_arrows: Vec::new(),
                    paths: Vec::new(),
                    path_points: Vec::new(),
                    scene_transitions: Vec::new(),
                    map_events: Vec::new(),
                    demo_archive_banks: Vec::new(),
                    actor_placements: Vec::new(),
                    treasure_placements: Vec::new(),
                    player_spawns: Vec::new(),
                },
            },
        ],
        message_flows: Vec::new(),
        ignored_archives: Vec::new(),
    }
}

fn entry_evidence() -> RuleEvidence {
    RuleEvidence {
        truth: TruthStatus::Established,
        records: vec![
            EvidenceRecord {
                id: "evidence.entry.actor".into(),
                kind: EvidenceKind::Extracted,
                source_sha256: Some(Digest([31; 32])),
                note: "Exact actor placement resource.".into(),
            },
            EvidenceRecord {
                id: "evidence.entry.message".into(),
                kind: EvidenceKind::Extracted,
                source_sha256: Some(Digest([23; 32])),
                note: "Exact message resource.".into(),
            },
            EvidenceRecord {
                id: "evidence.entry.stage".into(),
                kind: EvidenceKind::Extracted,
                source_sha256: Some(Digest([34; 32])),
                note: "Exact STAG message-group resource.".into(),
            },
            EvidenceRecord {
                id: "evidence.entry.presentation-caller".into(),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(Digest([35; 32])),
                note: "Source-audited presentation caller and generic item dispatch.".into(),
            },
        ],
    }
}

#[test]
fn compiled_set_round_trips_and_merges_transactionally() {
    let set = compiled_set();
    set.validate().unwrap();
    assert_eq!(set.mechanics.transitions.len(), 2);
    assert_eq!(
        CompiledMessageFlowSet::decode_canonical(&set.canonical_bytes().unwrap()).unwrap(),
        set
    );
    let mut facts = empty_facts();
    let mut mechanics = empty_mechanics();
    set.merge_into(&mut facts, &mut mechanics).unwrap();
    assert_eq!(mechanics.transitions.len(), 2);
    let facts_before = facts.clone();
    let mechanics_before = mechanics.clone();
    assert!(set.merge_into(&mut facts, &mut mechanics).is_err());
    assert_eq!(facts, facts_before);
    assert_eq!(mechanics, mechanics_before);
}

#[test]
fn overlay_contracts_are_digest_pinned_and_cleanup_is_conditional() {
    let overlay = MessageFlowResourceOverlaySet {
        schema: MESSAGE_FLOW_RESOURCE_OVERLAY_SET_SCHEMA.into(),
        id: "fixture-overlays".into(),
        import_profile_sha256: Digest([5; 32]),
        resources: vec![MessageFlowResourceOverlay {
            message_group: 3,
            resource_sha256: Digest([23; 32]),
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
                evidence: evidence(),
            }],
            cleanup_edges: vec![MessageCleanupEdge {
                transition_id: "transition.cleanup.message".into(),
                label: "Message cleanup".into(),
                approach_id: "approach.cleanup.message".into(),
                activation: PredicateExpression::Compare {
                    left: ValueReference::RuntimeLanguage,
                    operator: ComparisonOperator::Equal,
                    right: ValueReference::Literal {
                        value: StateValue::Text("en".into()),
                    },
                },
                packed_backing_coordinates: vec![0x0004],
                evidence: evidence(),
            }],
        }],
    };
    overlay.validate().unwrap();
    assert_eq!(
        MessageFlowResourceOverlaySet::decode_canonical(&overlay.canonical_bytes().unwrap())
            .unwrap(),
        overlay
    );
    let mut unconditional = overlay.clone();
    unconditional.resources[0].cleanup_edges[0].activation = PredicateExpression::True;
    assert_eq!(
        unconditional.validate().unwrap_err().field(),
        "message_flow_resource_overlay.cleanup.activation"
    );
    let mut unknown_cleanup = overlay;
    unknown_cleanup.resources[0].cleanup_edges[0].evidence.truth = TruthStatus::Unknown;
    assert_eq!(
        unknown_cleanup.validate().unwrap_err().field(),
        "message_flow_resource_overlay.cleanup.evidence"
    );
}

#[test]
fn tampered_merged_catalog_is_rejected() {
    let mut set = compiled_set();
    set.mechanics.transitions.pop();
    assert_eq!(
        set.validate().unwrap_err().field(),
        "compiled_message_flow_set.catalogs"
    );
}

#[test]
fn bundled_lanayru_entry_pins_exact_stage_actor_switch_and_flow() {
    let set = bundled_gz2e01_english_lanayru_entry_contracts().unwrap();
    assert_eq!(set.id, "gz2e01-en-lanayru-message-entries");
    assert_eq!(
        set.compiled_message_flow_set_schema,
        COMPILED_MESSAGE_FLOW_SET_SCHEMA
    );
    assert_eq!(
        set.compiled_message_flow_set_sha256.to_string(),
        "82bb787e1383dee2dc88937581252a10428f3f714a18d49c2d71f182702ef867"
    );
    let entry = &set.entries[0];
    assert_eq!(entry.message_group, 8);
    assert_eq!(entry.flow_id, 21);
    assert_eq!(entry.source_stage, "F_SP115");
    assert_eq!(entry.source_room, Some(1));
    assert_eq!(entry.source_layer, Some(13));
    let placement = entry.speaker.placement.as_ref().unwrap();
    assert_eq!(placement.archive_path, "files/res/Stage/F_SP115/R01_00.arc");
    assert_eq!(placement.chunk_tag, "ACTd");
    assert_eq!(placement.record_index, 0);
    assert_eq!(placement.actor_name, "Seirei");
    assert_eq!(entry.obligations.len(), 1);
    assert_eq!(entry.unknown_requirements.len(), 1);
    assert_eq!(set.presentation_requests.len(), 1);
    let request = &set.presentation_requests[0];
    assert_eq!(request.source_entry_id, entry.id);
    assert_eq!(request.event_id, 1);
    assert_eq!(request.item_id, 0xa3);
    assert_eq!(request.recent_item_target.component_id, "event-recent-item");
    assert!(matches!(
        &entry.additional_hard_guard,
        PredicateExpression::Compare {
            left: ValueReference::BoundRawBits {
                component_kind: crate::state::ComponentKind::DungeonMemory,
                binding: crate::state::ComponentBindingReference::CurrentStage,
                byte_offset: 10,
                byte_width: 1,
                mask: 16,
            },
            operator: ComparisonOperator::Equal,
            right: ValueReference::Literal {
                value: StateValue::Unsigned(16),
            },
        }
    ));
}

#[test]
fn actor_entry_contract_joins_exact_stage_actor_and_message_label() {
    let bundle = entry_bundle();
    bundle.validate().unwrap();
    let compiled = compiled_set_for_content(bundle.content.digest().unwrap());
    let entry = MessageFlowEntryContract {
        id: "gor-coron.fixture-flow-7".into(),
        label: "Talk to Gor Coron".into(),
        message_group: 3,
        resource_sha256: Digest([23; 32]),
        flow_id: 7,
        source_stage: "F_SP103".into(),
        source_room: Some(0),
        source_layer: Some(2),
        stage_archive_path: "files/res/Stage/F_SP103/Stage.arc".into(),
        stage_resource_sha256: Digest([34; 32]),
        speaker: MessageSpeakerContext {
            instance_id: Some("actor.gor-coron".into()),
            placement: Some(MessageSpeakerPlacement {
                archive_path: "files/res/Stage/F_SP103/Room0.arc".into(),
                resource_sha256: Digest([31; 32]),
                chunk_tag: "ACTR".into(),
                record_index: 4,
                layer: Some(2),
                actor_name: "Npc_Gro".into(),
                raw_hex: "0011223344556677".into(),
            }),
            stage: "F_SP103".into(),
            room: 0,
            zone: Some(5),
        },
        additional_hard_guard: PredicateExpression::True,
        obligations: Vec::new(),
        unknown_requirements: vec![UnknownRequirement {
            id: "unknown.entry.gor-coron-interaction".into(),
            description: "The exact actor interaction activation remains unaudited.".into(),
            evidence: RuleEvidence {
                truth: TruthStatus::Unknown,
                records: entry_evidence().records,
            },
        }],
        evidence: entry_evidence(),
    };
    let set = MessageFlowEntryContractSet {
        schema: MESSAGE_FLOW_ENTRY_CONTRACT_SET_SCHEMA.into(),
        id: "fixture-entry-contracts".into(),
        compiled_message_flow_set_schema: COMPILED_MESSAGE_FLOW_SET_SCHEMA.into(),
        compiled_message_flow_set_sha256: compiled.digest().unwrap(),
        entries: vec![entry],
        presentation_requests: vec![MessagePresentationRequestContract {
            id: "gor-coron.fixture-presentation".into(),
            label: "Gor Coron attempts item presentation".into(),
            source_entry_id: "gor-coron.fixture-flow-7".into(),
            event_id: 1,
            item_id: 7,
            recent_item_target: ComponentFieldTarget {
                component_id: "event-recent-item".into(),
                field: "get_item_no".into(),
            },
        }],
    };
    assert_eq!(
        MessageFlowEntryContractSet::decode_canonical(&set.canonical_bytes().unwrap()).unwrap(),
        set
    );
    let artifact = set.compile(&bundle, &compiled).unwrap();
    assert_eq!(
        CompiledMessageFlowEntrySet::decode_canonical(&artifact.canonical_bytes().unwrap())
            .unwrap(),
        artifact
    );
    assert_eq!(artifact.resolved_generic_item_grants.len(), 1);
    assert_eq!(artifact.resolved_generic_item_grants[0].item_id, 7);
    assert_eq!(artifact.mechanics.transitions.len(), 3);
    assert_eq!(artifact.mechanics.obligations.len(), 1);
    assert_eq!(artifact.mechanics.readers.len(), 4);
    let transition = artifact
        .mechanics
        .transitions
        .iter()
        .find(|transition| transition.id.contains("message-entry"))
        .unwrap();
    assert_eq!(transition.activation.unknown_requirements.len(), 1);
    assert!(transition.activation.effects.iter().any(|effect| matches!(
        effect,
        StateOperation::Write { target, value: StateValue::Signed(5) }
            if target.component_id == "flow.active-message" && target.field == "speaker_zone"
    )));
    let request = artifact
        .mechanics
        .transitions
        .iter()
        .find(|transition| transition.id.contains("message-presentation-request"))
        .unwrap();
    assert!(matches!(
        request.activation.effects.as_slice(),
        [StateOperation::CopyValue { source, target }]
            if source.component_id == "flow.active-message"
                && source.field == "item_id"
                && target.component_id == "event-recent-item"
    ));
    let grant = artifact
        .mechanics
        .transitions
        .iter()
        .find(|transition| transition.id.contains("generic-get-item"))
        .unwrap();
    assert_eq!(grant.activation.physical_obligation_ids.len(), 1);
    assert!(matches!(
        grant.activation.effects.as_slice(),
        [StateOperation::WriteBoundRaw {
            component_kind: ComponentKind::Inventory,
            binding: ComponentBindingReference::ActiveRuntimeFile,
            byte_offset: 0,
            mask,
            value,
        }] if mask == &[0x80] && value == &[0x80]
    ));
    assert!(transition.activation.effects.iter().any(|effect| matches!(
        effect,
        StateOperation::AdvanceFlow { flow_component_id, node_id }
            if flow_component_id == "flow.active-message" && node_id.starts_with("message-node.")
    )));

    let mut base = empty_mechanics();
    artifact.merge_into(&mut base).unwrap();
    assert_eq!(base.transitions.len(), 3);
    let before = base.clone();
    assert!(artifact.merge_into(&mut base).is_err());
    assert_eq!(base, before);

    let mut tampered = artifact.clone();
    tampered.mechanics.transitions[0].activation.effects.pop();
    assert_eq!(
        tampered.validate().unwrap_err().field(),
        "compiled_message_flow_entry_set.mechanics"
    );

    let mut wrong_actor = set.clone();
    wrong_actor.entries[0]
        .speaker
        .placement
        .as_mut()
        .unwrap()
        .raw_hex = "ffffffffffffffff".into();
    assert_eq!(
        wrong_actor.compile(&bundle, &compiled).unwrap_err().field(),
        "message_flow_entry_contract.speaker.placement"
    );

    let mut wrong_room = set.clone();
    wrong_room.entries[0].source_room = Some(1);
    assert_eq!(
        wrong_room.validate().unwrap_err().field(),
        "message_flow_entry_contract.speaker.room"
    );
    let mut wrong_layer = set;
    wrong_layer.entries[0].source_layer = Some(3);
    assert_eq!(
        wrong_layer.validate().unwrap_err().field(),
        "message_flow_entry_contract.speaker.placement.layer"
    );
}
