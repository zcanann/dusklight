//! Exact GZ2E01 reset-to-opening and opening-phase initialization mechanics.
//!
//! Later title input, name/file-select, slot-load, void, and death branches
//! remain separate audit targets.

use crate::PlannerContractError;
use crate::artifact::Digest;
use crate::identity::{ContentIdentity, ContextSelector, ExactContext, RuntimeConfiguration};
use crate::logic::{
    ComparisonOperator, ContextScope, EvidenceKind, EvidenceRecord, PredicateExpression,
    RuleEvidence, TruthStatus, ValueReference,
};
use crate::return_place::{GZ2E01_CONTENT_SHA256, GZ2E01_EN_RUNTIME_SHA256};
use crate::state::{
    BackingAttachment, ComponentBinding, ComponentKind, ComponentPayload, ComponentProvenance,
    ComponentSelector, ExecutionContext, PhysicalSlotId, ProvenanceSourceKind, RuntimeFileOrigin,
    SceneLocation, SemanticLifetime, SerializationOwner, StateComponent, StateValue,
};
use crate::transition::{
    ActivationContract, CandidateTransition, ComponentFieldTarget, MECHANICS_CATALOG_SCHEMA,
    MechanicsCatalog, StateOperation, TransitionKind,
};
use std::collections::BTreeMap;

const RESET_CONTROL_COMPONENT: &str = "reset-control";
const RESTART_COMPONENT: &str = "restart";
const OPENING_PROCESS_CONTROL_COMPONENT: &str = "opening-process-control";
const TITLE_CONTROL_COMPONENT: &str = "title-control";
const NAME_SCENE_CONTROL_COMPONENT: &str = "name-scene-control";
const RUNTIME_FILE_HEADER_COMPONENT: &str = "runtime-file.header";
const PERSISTENT_EVENT_COMPONENT: &str = "flags.persistent-event-registers";
const OBSERVED_EVENT_COMPONENT: &str = "flags.event";
const LIGHT_DROP_COMPONENT: &str = "save.player-light-drop";
const OBSERVED_TEMPORARY_COMPONENT: &str = "flags.temporary";
const TEMPORARY_EVENT_COMPONENT: &str = "flags.temporary-event-registers";
const DUNGEON_SESSION_LABEL_COMPONENT: &str = "flags.dungeon-session-labels";
const LOADED_STAGE_MEMORY_COMPONENT: &str = "flags.loaded-stage-memory";
const ROOM_SWITCH_LABEL_COMPONENT: &str = "flags.room-switch-labels";
const INVENTORY_COMPONENT: &str = "inventory-and-resources";
const RETURN_PLACE_COMPONENT: &str = "return-place";
const FILE_SELECT_BUFFER_OWNER_PREFIX: &str = "file-select-buffer.slot";

/// Compiles the exact successful prefix of `dComIfG_resetToOpening` for
/// GZ2E01. It records the scheduled opening process/load and the restart-room
/// parameter write without pretending that the pending F_SP102 request is an
/// already loaded, traversable world location.
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
    let evidence = RuleEvidence {
        truth: TruthStatus::Established,
        records: vec![
            EvidenceRecord {
                id: "binary.gz2e01.dcomifg-reset-to-opening".into(),
                kind: EvidenceKind::Extracted,
                source_sha256: Some(parse_digest(
                    "bde63a102b6502e418e5a8c53cff364f66f6510420a7316a492664ab7530e28d",
                )),
                note: "Canonical exact-DOL function artifact: VA 0x8002cd44, size 0x74, code SHA-256 3cc637771d531950401a332a83b90296df2b5aa9bec6cc292ad5546fec23df30.".into(),
            },
            EvidenceRecord {
                id: "binary.gz2e01.dcomifg-change-opening-scene".into(),
                kind: EvidenceKind::Extracted,
                source_sha256: Some(parse_digest(
                    "658f63b09b0f43dcb5b2662dbbf140de889fe19374dac8ccee32d9545ac2d781",
                )),
                note: "Canonical exact-DOL function artifact: VA 0x8002cc54, size 0xf0, code SHA-256 0b5c465a32ffb343d9863e04970f5c2621a5bb0b854efc974708fb0229828a41.".into(),
            },
            EvidenceRecord {
                id: "source.gcn-reset-to-opening-prefix".into(),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(parse_digest(
                    "b9b37aed0b76eef2d27b35a2ece6ee077086a970f98d18936a83649303f15761",
                )),
                note: "Source-family audit establishes the GCN guards, F_SP102/start 100/room 0/layer 10 pending load, PROC_OPENING_SCENE request, and restart-room parameter zero write.".into(),
            },
        ],
    };
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
    let opening_evidence = RuleEvidence {
        truth: TruthStatus::Established,
        records: vec![
            exact_function_evidence(
                "binary.gz2e01.opening-phase-4",
                "caf6f662835287e2c74e341b2771e142c8b0a1dd6da7745775a01f1a36cb62cc",
                "phase_4__FP9dScnPly_c at VA 0x8025a654, size 0x3a0, code SHA-256 5e116171d689fcf368218490f24009dd176205648fd30b697bdab3a7efb179aa.",
            ),
            exact_function_evidence(
                "binary.gz2e01.dsv-info-init",
                "433224e88c9c58df6d5abd49863e2a871a965f2806288e1d19fd36f1e267d93b",
                "init__10dSv_info_cFv at VA 0x80034fcc, size 0x50, code SHA-256 5c80b3dba87ae8f968b5e4620f0872d4355358debc63d5556adba4b8d3d4338d.",
            ),
            exact_function_evidence(
                "binary.gz2e01.dsv-player-init",
                "0bc0b6246b3a6cad9a8a0409ef59358fa544632ac5884b27008a3e5dd4db185b",
                "init__12dSv_player_cFv at VA 0x800346a4, size 0xac, code SHA-256 668f452c16c5ed413535588b00c5a497b236a29f7e52f55c521b58e179968766.",
            ),
            exact_function_evidence(
                "binary.gz2e01.dsv-save-init",
                "a9953253f543fbdc9d0998e6f369fb2f0bac45b411c44baee5ff9fd34fccda9b",
                "init__10dSv_save_cFv at VA 0x8003501c, size 0x8c, code SHA-256 e405d830e4f445c950fb158ddf8f6107430524a2708d82bd1b31c7e13e804d48.",
            ),
            exact_function_evidence(
                "binary.gz2e01.empty-initial-event-hook",
                "c40daaee608a8afd5c471d54a1a87efe7eb42695036729215a3fa413d256892f",
                "setInitEventBit__Fv at VA 0x80035c88 is an exact four-byte immediate return, code SHA-256 f332ea5b5437103cbb6f1508679da89eec9288ad775c96c439a17fccabe3de8e.",
            ),
            exact_function_evidence(
                "binary.gz2e01.player-return-place-init",
                "0eeb93826008824d6810499ce61ec1c8e8065c7a06c8a9576022b76532f75917",
                "init__25dSv_player_return_place_cFv at VA 0x80032cc8, size 0x54, code SHA-256 252007ca2690e54e6a13019527739c4e55dff0f1ac1e7ec6ff8b1d425ed6ab87.",
            ),
            exact_function_evidence(
                "binary.gz2e01.select-equip-shield",
                "7a7920012416bdf116d20be436514da59bf00da2e6cbab28dcc0842e33078a23",
                "dComIfGs_setSelectEquipShield__FUc at VA 0x8002ef94, size 0xac, code SHA-256 beeb64d1fa6897f83de2674e9053189416486ca4066c39d1efb4e647bf7c7e14.",
            ),
            exact_function_evidence(
                "binary.gz2e01.select-equip-sword",
                "1d014bd60aa88951beb555a13853be0068f91790989639909bcff8a088decd9e",
                "dComIfGs_setSelectEquipSword__FUc at VA 0x8002eec0, size 0xd4, code SHA-256 b0cdfc30b3f91a906cf4c8066f8eb5ec7055df50de7ade590c5c721ea0732761.",
            ),
            EvidenceRecord {
                id: "source.gz2e01.opening-file0-initialization".into(),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(parse_digest(
                    "c8f30a83c45d6c42078945b09f6e4e3459c832184e641ff442fa7d0e49258077",
                )),
                note: "Opening phase 4 initializes dSv_info, life, Kokiri clothes, Ordon sword, Hylian shield, and event 0x0601. Sword/shield setters set collection masks but off-item-bit=false leaves acquisition bits clear.".into(),
            },
            EvidenceRecord {
                id: "source.gz2e01.save-domain-initializers".into(),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(parse_digest(
                    "7e6f09aa36af30932e8ce64423284f885ed0b4e632b22f18d6f0a6b4d104b453",
                )),
                note: "dSv_info_c::init resets savedata, live stage memory, dungeon memory, zones, and temporary event state; nested player initialization establishes the exact retained fields published here.".into(),
            },
        ],
    };
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
    let title_evidence = RuleEvidence {
        truth: TruthStatus::Established,
        records: vec![EvidenceRecord {
            id: "source.gz2e01.title-key-and-name-scene-request".into(),
            kind: EvidenceKind::SourceAudited,
            source_sha256: Some(parse_digest(
                "39378bcbc78e5ffae3287f127cc48cd2c22e18723cf31cfeb5bd84a2becdc4cb",
            )),
            note: "GZ2E01 source audit: title keyWait accepts A/Start, advances to nextScene, and nextScene requests PROC_NAME_SCENE only while reset and overlap-peek are clear.".into(),
        }],
    };
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
    let file_select_evidence = RuleEvidence {
        truth: TruthStatus::Established,
        records: vec![
            EvidenceRecord {
                id: "source.gz2e01.name-scene-create".into(),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(parse_digest(
                    "f095894aabc198c068ee0ac9872f6c277c0e035b36c4d29d1f896e7c2eb0fe4b",
                )),
                note: "GZ2E01 source audit: the normal name-scene create path constructs file select, then writes mNoFile = 0.".into(),
            },
            EvidenceRecord {
                id: "source.gz2e01.file-select-create".into(),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(parse_digest(
                    "aee1cb134ec92953fd04dc321f4dae5f5c98ed1d2e766d1306a70d932294eb0d",
                )),
                note: "GZ2E01 source audit: dFile_select_c::_create runs dComIfGs_init and then writes mNewFile = 0 before the name scene enters file-select-open.".into(),
            },
        ],
    };
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
    let file_select_branch_evidence = RuleEvidence {
        truth: TruthStatus::Established,
        records: vec![
            EvidenceRecord {
                id: "source.gz2e01.file-select-branches".into(),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(parse_digest(
                    "aee1cb134ec92953fd04dc321f4dae5f5c98ed1d2e766d1306a70d932294eb0d",
                )),
                note: "GZ2E01 file-select audit separates blank-slot mNewFile/mDataNum writes, existing-slot Start/card_to_memory, and no-save buffer initialization/card_to_memory/header writes.".into(),
            },
            EvidenceRecord {
                id: "source.gz2e01.card-to-memory".into(),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(parse_digest(
                    "7e6f09aa36af30932e8ce64423284f885ed0b4e632b22f18d6f0a6b4d104b453",
                )),
                note: "dSv_info_c::card_to_memory copies dSv_save_c only, then performs load-time life/key/item-layout normalization; live header and other non-save runtime metadata are outside that projection.".into(),
            },
        ],
    };
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
                        pending_compare(
                            name_field("menu_command_raw"),
                            StateValue::Unsigned(1),
                        ),
                        pending_compare(
                            ValueReference::PhysicalSlotImageAvailable {
                                slot: PhysicalSlotId(slot as u8),
                            },
                            StateValue::Boolean(true),
                        ),
                    ],
                },
                physical_obligation_ids: Vec::new(),
                effects: vec![
                    StateOperation::LoadActiveRuntimeFromSlot {
                        source_slot: PhysicalSlotId(slot as u8),
                        destination_id_suffix: format!("file-select-slot-{slot}"),
                        destination_allowed_serialization_targets: vec![
                            PhysicalSlotId(1),
                            PhysicalSlotId(2),
                            PhysicalSlotId(3),
                        ],
                        carried_runtime_component_ids: carried_runtime_component_ids.clone(),
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
                            component_id: NAME_SCENE_CONTROL_COMPONENT.into(),
                            field: "phase".into(),
                        },
                        value: StateValue::Text("selection_end".into()),
                    },
                ],
                unknown_requirements: vec![UnknownRequirement {
                    id: "unknown.file-select-card-to-memory-normalization".into(),
                    description: "Project the life clamp, dungeon-6 key clear, hookshot slot rewrites, lineup rebuild, vibration, and save-stage display writes performed after the selected dSv_save_c copy.".into(),
                    evidence: RuleEvidence {
                        truth: TruthStatus::Unknown,
                        records: file_select_branch_evidence.records.clone(),
                    },
                }],
            },
            evidence: file_select_branch_evidence.clone(),
        });
    }
    let initialized_buffer_component_ids = vec![
        PERSISTENT_EVENT_COMPONENT.into(),
        INVENTORY_COMPONENT.into(),
        RETURN_PLACE_COMPONENT.into(),
        LIGHT_DROP_COMPONENT.into(),
    ];
    let mut no_card_effects = (1_u8..=3)
        .map(|slot| StateOperation::ReplaceCustomStore {
            owner: file_select_buffer_owner(slot),
            components: initialized_file_select_buffer(slot),
        })
        .collect::<Vec<_>>();
    no_card_effects.extend([
        StateOperation::RestorePayloadsFromCustomStore {
            owner: file_select_buffer_owner(1),
            component_ids: initialized_buffer_component_ids,
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
    let play_scene_request_evidence = RuleEvidence {
        truth: TruthStatus::Established,
        records: vec![
            EvidenceRecord {
                id: "source.gz2e01.name-scene-change-game-scene".into(),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(parse_digest(
                    "f095894aabc198c068ee0ac9872f6c277c0e035b36c4d29d1f896e7c2eb0fe4b",
                )),
                note: "dScnName_c::changeGameScene calls dComIfGs_gameStart, overrides a new file's next stage with F_SP108/room 1/spawn 21/layer 13, and requests PROC_PLAY_SCENE without proving process or world activation.".into(),
            },
            EvidenceRecord {
                id: "source.gz2e01.game-start-return-place".into(),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(parse_digest(
                    "b9b37aed0b76eef2d27b35a2ece6ee077086a970f98d18936a83649303f15761",
                )),
                note: "dComIfGs_gameStart requests the structured player return place with layer -1 before the new-file branch optionally overrides it.".into(),
            },
        ],
    };
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
        enter_and_initialize_transition,
        opening_transition,
        reset_transition,
        title_key_accept_transition,
        title_request_name_scene_transition,
    ];
    transitions.extend(file_select_branch_transitions);
    transitions.sort_by(|left, right| left.id.cmp(&right.id));
    let catalog = MechanicsCatalog {
        schema: MECHANICS_CATALOG_SCHEMA.into(),
        transitions: vec![
            name_scene_file_select_transition,
            enter_and_initialize_transition,
            opening_transition,
            reset_transition,
            title_key_accept_transition,
            title_request_name_scene_transition,
        ],
        obligations: Vec::new(),
        writers: Vec::new(),
        gates: Vec::new(),
        readers: Vec::new(),
        reconstruction_rules: Vec::new(),
        obstructions: Vec::new(),
        resolvers: Vec::new(),
        techniques: Vec::new(),
        microtraces: Vec::new(),
        goals: Vec::new(),
    };
    catalog.validate()?;
    Ok(catalog)
}

fn parse_digest(value: &str) -> Digest {
    value.parse().expect("compile-time SHA-256 literal")
}

fn exact_function_evidence(id: &str, artifact_sha256: &str, note: &str) -> EvidenceRecord {
    EvidenceRecord {
        id: id.into(),
        kind: EvidenceKind::Extracted,
        source_sha256: Some(parse_digest(artifact_sha256)),
        note: note.into(),
    }
}

fn dcomifgs_init_effects() -> Vec<StateOperation> {
    let mut loaded_stage_known_mask = vec![0xff; 0x20];
    // dSv_memBit_c::init writes bytes 0x00..0x1d. Its two tail-padding
    // bytes are not written and therefore remain explicitly unknown.
    loaded_stage_known_mask[0x1e] = 0;
    loaded_stage_known_mask[0x1f] = 0;
    vec![
        StateOperation::InvalidatePayloads {
            selector: ComponentSelector::Kind {
                component_kind: ComponentKind::DungeonMemory,
            },
            include_active_runtime_serialized_stores: true,
        },
        StateOperation::ReplacePayload {
            component_id: LOADED_STAGE_MEMORY_COMPONENT.into(),
            payload: ComponentPayload::Raw {
                bytes: vec![0; 0x20],
                known_mask: loaded_stage_known_mask,
            },
        },
        StateOperation::ReplacePayload {
            component_id: PERSISTENT_EVENT_COMPONENT.into(),
            payload: ComponentPayload::Raw {
                bytes: vec![0; 256],
                known_mask: vec![0xff; 256],
            },
        },
        StateOperation::ReplacePayload {
            component_id: OBSERVED_EVENT_COMPONENT.into(),
            payload: ComponentPayload::Unknown {
                expected_bytes: None,
            },
        },
        StateOperation::ReplacePayload {
            component_id: LIGHT_DROP_COMPONENT.into(),
            payload: ComponentPayload::Raw {
                bytes: vec![0; 5],
                known_mask: vec![0xff; 5],
            },
        },
        StateOperation::ReplacePayload {
            component_id: OBSERVED_TEMPORARY_COMPONENT.into(),
            payload: ComponentPayload::Unknown {
                expected_bytes: None,
            },
        },
        StateOperation::ReplacePayload {
            component_id: TEMPORARY_EVENT_COMPONENT.into(),
            payload: ComponentPayload::Raw {
                bytes: vec![0; 256],
                known_mask: vec![0xff; 256],
            },
        },
        StateOperation::ReplacePayload {
            component_id: DUNGEON_SESSION_LABEL_COMPONENT.into(),
            payload: ComponentPayload::Unknown {
                expected_bytes: None,
            },
        },
        StateOperation::ReplacePayload {
            component_id: ROOM_SWITCH_LABEL_COMPONENT.into(),
            payload: ComponentPayload::Unknown {
                expected_bytes: None,
            },
        },
        StateOperation::ReplacePayload {
            component_id: RETURN_PLACE_COMPONENT.into(),
            payload: ComponentPayload::Structured {
                fields: BTreeMap::from([
                    ("player_status".into(), StateValue::Unsigned(0)),
                    ("room".into(), StateValue::Signed(1)),
                    ("stage".into(), StateValue::Text("F_SP108".into())),
                ]),
            },
        },
        StateOperation::ReplacePayload {
            component_id: INVENTORY_COMPONENT.into(),
            payload: base_inventory_payload(),
        },
    ]
}

fn base_inventory_payload() -> ComponentPayload {
    ComponentPayload::Structured {
        fields: BTreeMap::from([
            ("maximum_life".into(), StateValue::Unsigned(15)),
            ("life".into(), StateValue::Unsigned(12)),
            ("rupees".into(), StateValue::Unsigned(0)),
            ("inventory".into(), StateValue::Bytes(vec![0xff; 24])),
            ("selected_items".into(), StateValue::Bytes(vec![0xff; 4])),
            ("mixed_items".into(), StateValue::Bytes(vec![0xff; 4])),
            (
                "equipment".into(),
                StateValue::Bytes(vec![0x2e, 0xff, 0xff, 0xff, 0xff, 0]),
            ),
            ("bomb_counts".into(), StateValue::Bytes(vec![0; 3])),
            (
                "bomb_capacities".into(),
                StateValue::Bytes(vec![30, 15, 10]),
            ),
            ("bottle_quantities".into(), StateValue::Bytes(vec![0; 4])),
            ("acquired_item_bits".into(), StateValue::Bytes(vec![0; 32])),
            ("collect_item_bits".into(), StateValue::Bytes(vec![0; 8])),
        ]),
    }
}

fn file_select_buffer_owner(slot: u8) -> SerializationOwner {
    SerializationOwner::Custom {
        id: format!("{FILE_SELECT_BUFFER_OWNER_PREFIX}-{slot}"),
    }
}

fn initialized_file_select_buffer(slot: u8) -> Vec<StateComponent> {
    let owner = file_select_buffer_owner(slot);
    let binding = ComponentBinding::Custom {
        kind_id: "file-select-save-buffer".into(),
        context_id: format!("slot-{slot}"),
    };
    let provenance = || {
        vec![ComponentProvenance {
            source_kind: ProvenanceSourceKind::Initialized,
            source_id: "source.gz2e01.initdata-to-card".into(),
            source_sha256: Some(parse_digest(
                "7e6f09aa36af30932e8ce64423284f885ed0b4e632b22f18d6f0a6b4d104b453",
            )),
            transition_id: None,
        }]
    };
    let component =
        |id: &str, component_kind: ComponentKind, payload: ComponentPayload| StateComponent {
            id: id.into(),
            component_kind,
            payload,
            binding: binding.clone(),
            lifetime: SemanticLifetime::Session,
            serialization_owner: owner.clone(),
            provenance: provenance(),
        };
    vec![
        component(
            PERSISTENT_EVENT_COMPONENT,
            ComponentKind::Custom {
                id: "persistent-event-registers".into(),
            },
            ComponentPayload::Raw {
                bytes: vec![0; 256],
                known_mask: vec![0xff; 256],
            },
        ),
        component(
            INVENTORY_COMPONENT,
            ComponentKind::Inventory,
            base_inventory_payload(),
        ),
        component(
            RETURN_PLACE_COMPONENT,
            ComponentKind::PersistentSave,
            ComponentPayload::Structured {
                fields: BTreeMap::from([
                    ("player_status".into(), StateValue::Unsigned(0)),
                    ("room".into(), StateValue::Signed(1)),
                    ("stage".into(), StateValue::Text("F_SP108".into())),
                ]),
            },
        ),
        component(
            LIGHT_DROP_COMPONENT,
            ComponentKind::Custom {
                id: "player-light-drop".into(),
            },
            ComponentPayload::Raw {
                bytes: vec![0; 5],
                known_mask: vec![0xff; 5],
            },
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evaluation::{
        EvaluatedTruth, EvidencePolicy, FeasibilityMode, PredicateEvaluator,
        TransitionClassification,
    };
    use crate::execution::PlannerExecutionState;
    use crate::identity::{
        CONTENT_IDENTITY_SCHEMA, ContentFingerprint, GamePlatform, GameRegion,
        RUNTIME_CONFIGURATION_SCHEMA,
    };
    use crate::logic::{FACT_CATALOG_SCHEMA, FactCatalog};
    use crate::snapshot::{STATE_SNAPSHOT_SCHEMA, StateDiff, StateSnapshot};
    use crate::state::{
        ActorLifecycle, BackingAttachment, CaptureStatus, ComponentBinding,
        ComponentBindingReference, ComponentKind, ComponentPayload, ComponentProvenance,
        EXECUTION_ENVIRONMENT_SCHEMA, ExecutionEnvironment, LiveWorldObject,
        PhysicalSlotObservation, PlayerForm, PlayerState, ProvenanceSourceKind, RuntimeFile,
        RuntimeFileLifecycle, RuntimeFileOrigin, SemanticLifetime, SerializationOwner,
        StateComponent,
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

    fn fields_for<'a>(
        state: &'a PlannerExecutionState,
        id: &str,
    ) -> &'a BTreeMap<String, StateValue> {
        let ComponentPayload::Structured { fields } = &component_for(state, id).payload else {
            panic!("{id} should be structured")
        };
        fields
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
                    component(
                        INVENTORY_COMPONENT,
                        ComponentKind::Inventory,
                        [("life", StateValue::Unsigned(80))],
                    ),
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
                    raw_component(
                        LIGHT_DROP_COMPONENT,
                        ComponentKind::Custom {
                            id: "player-light-drop".into(),
                        },
                        5,
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
            .find(|transition| {
                transition.id == "transition.gz2e01.opening-enter-and-initialize-file0"
            })
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
                        SerializationOwner::RuntimeFile { .. }
                            | SerializationOwner::StageBank { .. }
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
        set_structured_field(
            &mut blank,
            NAME_SCENE_CONTROL_COMPONENT,
            "phase",
            StateValue::Text("selection_end".into()),
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
            TransitionClassification::FeasibilityUnknown,
            "the backing-store load is modeled, but exact post-copy normalization remains explicit"
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
            StateValue::Unsigned(80),
            "the selected sealed image replaces the title initializer payload"
        );
        assert_eq!(
            fields_for(&existing, RUNTIME_FILE_HEADER_COMPONENT)["data_num_raw"],
            StateValue::Unsigned(0)
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
}
