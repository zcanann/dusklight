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
    MechanicsCatalog, SaveProjectionOperation, StateOperation, TransitionKind,
};
use std::collections::BTreeMap;

const RESET_CONTROL_COMPONENT: &str = "reset-control";
const RESTART_COMPONENT: &str = "restart";
const OPENING_PROCESS_CONTROL_COMPONENT: &str = "opening-process-control";
const TITLE_CONTROL_COMPONENT: &str = "title-control";
const NAME_SCENE_CONTROL_COMPONENT: &str = "name-scene-control";
const SAVE_MENU_CONTROL_COMPONENT: &str = "save-menu-control";
const RUNTIME_FILE_HEADER_COMPONENT: &str = "runtime-file.header";
const PERSISTENT_EVENT_COMPONENT: &str = "flags.persistent-event-registers";
const OBSERVED_EVENT_COMPONENT: &str = "flags.event";
const LIGHT_DROP_COMPONENT: &str = "save.player-light-drop";
const PLAYER_INFO_COMPONENT: &str = "save.player-info";
const OBSERVED_TEMPORARY_COMPONENT: &str = "flags.temporary";
const TEMPORARY_EVENT_COMPONENT: &str = "flags.temporary-event-registers";
const DUNGEON_SESSION_LABEL_COMPONENT: &str = "flags.dungeon-session-labels";
const LOADED_STAGE_MEMORY_COMPONENT: &str = "flags.loaded-stage-memory";
const DUNGEON_SIX_SAVE_COMPONENT: &str = "save.dungeon-memory.index-6";
const ROOM_SWITCH_LABEL_COMPONENT: &str = "flags.room-switch-labels";
const INVENTORY_COMPONENT: &str = "inventory-and-resources";
const RETURN_PLACE_COMPONENT: &str = "return-place";
const ACTIVE_VIBRATION_COMPONENT: &str = "session.active-vibration";
const SAVE_STAGE_DISPLAY_COMPONENT: &str = "session.save-stage-display";
const FILE_SELECT_BUFFER_OWNER_PREFIX: &str = "file-select-buffer.slot";

const ITEM_NONE: u8 = 0xff;
const ITEM_HOOKSHOT: u8 = 0x44;
const ITEM_DOUBLE_CLAWSHOT: u8 = 0x47;
const ITEM_LINEUP_ORDER: [u8; 23] = [
    10, 8, 6, 2, 9, 4, 3, 0, 1, 23, 20, 5, 15, 16, 17, 11, 12, 13, 14, 19, 18, 22, 21,
];
const DEFAULT_PLAYER_NAME_BYTES: &[u8] = b"Link\0";
const DEFAULT_HORSE_NAME_BYTES: &[u8] = b"Epona\0";

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
    let scheduler_evidence = RuleEvidence {
        truth: TruthStatus::Established,
        records: vec![EvidenceRecord {
            id: "source.gz2e01-title-process-activation".into(),
            kind: EvidenceKind::SourceAudited,
            source_sha256: Some(parse_digest(
                "f095894aabc198c068ee0ac9872f6c277c0e035b36c4d29d1f896e7c2eb0fe4b",
            )),
            note: "GZ2E01 process audit separates a submitted scene request from the scheduler-observed opening/name process create phase; these transitions record those independently observed activations.".into(),
        }],
    };
    let process_component = |id: &str,
                             component_kind: ComponentKind,
                             fields: BTreeMap<String, StateValue>| StateComponent {
        id: id.into(),
        component_kind,
        payload: ComponentPayload::Structured { fields },
        binding: ComponentBinding::Session {
            session_id: "process".into(),
        },
        lifetime: SemanticLifetime::Session,
        serialization_owner: SerializationOwner::None,
        provenance: vec![ComponentProvenance {
            source_kind: ProvenanceSourceKind::TraceObservation,
            source_id: "observation.gz2e01-process-activation".into(),
            source_sha256: Some(parse_digest(
                "f095894aabc198c068ee0ac9872f6c277c0e035b36c4d29d1f896e7c2eb0fe4b",
            )),
            transition_id: None,
        }],
    };
    let opening_process_activation_transition = CandidateTransition {
        id: "transition.gz2e01.observe-opening-phase-4".into(),
        label: "Observe opening process activation at phase 4".into(),
        scope: reset_transition.scope.clone(),
        transition_kind: TransitionKind::Other,
        approach_id: "scheduler.observe-opening-phase-4".into(),
        activation: ActivationContract {
            hard_guards: PredicateExpression::All {
                terms: vec![
                    compare(
                        ValueReference::ExecutionProcess,
                        ComparisonOperator::Equal,
                        StateValue::Text("PROC_OPENING_SCENE".into()),
                    ),
                    compare(
                        ValueReference::PendingWorldLoadStage,
                        ComparisonOperator::Equal,
                        StateValue::Text("F_SP102".into()),
                    ),
                    compare(
                        ValueReference::ComponentField {
                            component_id: RESET_CONTROL_COMPONENT.into(),
                            field: "opening_process_observed".into(),
                        },
                        ComparisonOperator::Equal,
                        StateValue::Boolean(false),
                    ),
                ],
            },
            physical_obligation_ids: Vec::new(),
            effects: vec![
                StateOperation::Initialize {
                    component: process_component(
                        OPENING_PROCESS_CONTROL_COMPONENT,
                        ComponentKind::Session,
                        BTreeMap::from([(
                            "phase".into(),
                            StateValue::Text("phase_4".into()),
                        )]),
                    ),
                },
                StateOperation::Initialize {
                    component: process_component(
                        TITLE_CONTROL_COMPONENT,
                        ComponentKind::Title,
                        BTreeMap::from([
                            ("phase".into(), StateValue::Text("key_wait".into())),
                            ("reset_requested".into(), StateValue::Boolean(false)),
                            ("overlap_peek".into(), StateValue::Boolean(false)),
                            ("a_triggered".into(), StateValue::Boolean(true)),
                            ("start_triggered".into(), StateValue::Boolean(false)),
                        ]),
                    ),
                },
                StateOperation::Write {
                    target: ComponentFieldTarget {
                        component_id: RESET_CONTROL_COMPONENT.into(),
                        field: "opening_process_observed".into(),
                    },
                    value: StateValue::Boolean(true),
                },
            ],
            unknown_requirements: Vec::new(),
        },
        evidence: scheduler_evidence.clone(),
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
    let name_scene_activation_transition = CandidateTransition {
        id: "transition.gz2e01.observe-name-scene-create".into(),
        label: "Observe name scene activation at file-select creation".into(),
        scope: reset_transition.scope.clone(),
        transition_kind: TransitionKind::Other,
        approach_id: "scheduler.observe-name-scene-create".into(),
        activation: ActivationContract {
            hard_guards: PredicateExpression::All {
                terms: vec![
                    pending_compare(
                        ValueReference::ExecutionProcess,
                        StateValue::Text("PROC_OPENING_SCENE".into()),
                    ),
                    pending_compare(
                        title_field("phase"),
                        StateValue::Text("scene_requested".into()),
                    ),
                    pending_compare(
                        ValueReference::ComponentField {
                            component_id: RESET_CONTROL_COMPONENT.into(),
                            field: "name_scene_observed".into(),
                        },
                        StateValue::Boolean(false),
                    ),
                ],
            },
            physical_obligation_ids: Vec::new(),
            effects: vec![
                StateOperation::Initialize {
                    component: process_component(
                        NAME_SCENE_CONTROL_COMPONENT,
                        ComponentKind::Title,
                        BTreeMap::from([(
                            "phase".into(),
                            StateValue::Text("create_file_select".into()),
                        )]),
                    ),
                },
                StateOperation::SetExecutionContext {
                    context: ExecutionContext::Process {
                        process_name: "PROC_NAME_SCENE".into(),
                        pending_world_load: None,
                    },
                },
                StateOperation::Write {
                    target: ComponentFieldTarget {
                        component_id: RESET_CONTROL_COMPONENT.into(),
                        field: "name_scene_observed".into(),
                    },
                    value: StateValue::Boolean(true),
                },
            ],
            unknown_requirements: Vec::new(),
        },
        evidence: scheduler_evidence,
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
            exact_function_evidence(
                "binary.gz2e01.card-to-memory",
                "fca390c69693273eab6336a9ce094473227ea9c98f4e13a627c12452ddc12352",
                "card_to_memory__10dSv_info_cFPci at VA 0x80035a04, size 0x1cc, code SHA-256 5f50141704f8daa60900f0559ef6f2272965b195fa673d29e73ceef82a593dc0.",
            ),
            exact_function_evidence(
                "binary.gz2e01.set-line-up-item",
                "f9edd7f12fcbce48fb6c07b036ae3018abb07d8fb1510044f848a05eacbf7a14",
                "setLineUpItem__17dSv_player_item_cFv at VA 0x800332f8, size 0x5c, code SHA-256 08c250dbed9821493d7a25ae234328a99fe912228b8ac54bcffe5314b5c1e323.",
            ),
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
                        (
                            "selected_entry_kind".into(),
                            StateValue::Text("new".into()),
                        ),
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
    let name_confirmation_evidence = RuleEvidence {
        truth: TruthStatus::Established,
        records: vec![
            EvidenceRecord {
                id: "source.gz2e01.file-select-name-confirmation".into(),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(parse_digest(
                    "aee1cb134ec92953fd04dc321f4dae5f5c98ed1d2e766d1306a70d932294eb0d",
                )),
                note: "Source audit establishes both name confirmations, default horse setup, both Back paths, and final mIsSelectEnd. These mutate live dSv_save_c player-info; no physical save API is called.".into(),
            },
            exact_function_evidence(
                "binary.gz2e01.file-select-name-input",
                "fd93ea0a72e1008434af10c19cd8f59a430f01bd8a044f5173bd97e78bd6ae0a",
                "nameInput__14dFile_select_cFv at VA 0x801873bc, size 0x13c, code SHA-256 0388366b478b3a51aa2a7cd4c7825eb7370dec67b14e3b7db98e2c9aad284ba5.",
            ),
            exact_function_evidence(
                "binary.gz2e01.file-select-name-input-fade",
                "ecb601568e64364a3adfc779bf737949371a1460c1daca3651ec31ef1631c726",
                "nameInputFade__14dFile_select_cFv at VA 0x8018759c, size 0x104, code SHA-256 1972401d18a34e1f1d8c6ab180df465df2c17d34a9fc03dbcdda37b1229249d8.",
            ),
            exact_function_evidence(
                "binary.gz2e01.file-select-name-input-2-move",
                "9da639084fa4d342c1154c2669aa65eb22c81d3fa52b9281f0ab100c15a86f33",
                "nameInput2Move__14dFile_select_cFv at VA 0x801876a0, size 0xac, code SHA-256 a96931c928651f29eea71bf214964abe46f8af5a7a3006581153fef732c614e5.",
            ),
            exact_function_evidence(
                "binary.gz2e01.file-select-name-input-2",
                "e7a2a4b3ed67e42938aa0a28f2deaa66edab757618d0bcacdaef3598e627cc13",
                "nameInput2__14dFile_select_cFv at VA 0x8018774c, size 0xd8, code SHA-256 32fb5e79113d0a52bde235fd8c1fb3c052b66445bc1b7264e8c065d53e5ea87b.",
            ),
        ],
    };
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
    let successful_save_evidence = RuleEvidence {
        truth: TruthStatus::Established,
        records: vec![
            EvidenceRecord {
                id: "source.gz2e01.save-menu-success".into(),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(parse_digest(
                    "78acd5de6255c5031eeeb0d041509b9080b7121e68a1546d14ba75a6454f0f4e",
                )),
                note: "dMenu_save_c dataWrite commits the current stage, projects the selected entry, checksums it, and submits the full buffer. Only SaveSync result 1 updates mDataNum/mNoFile and enters a success UI branch.".into(),
            },
            EvidenceRecord {
                id: "source.gz2e01.memory-to-card".into(),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(parse_digest(
                    "7e6f09aa36af30932e8ce64423284f885ed0b4e632b22f18d6f0a6b4d104b453",
                )),
                note: "memory_to_card copies dSv_save_c after temporary lantern normalization, then restores the live lantern/event values. The promoted neutral branch proves those temporary transforms are identity on projected fields.".into(),
            },
            exact_function_evidence(
                "binary.gz2e01.put-save",
                "eb3032a28f0a4d08684d74894785c1760a241020d907b12bee19e350eda1caf9",
                "putSave__10dSv_info_cFi at VA 0x800350f0, size 0x5c, code SHA-256 f94364f83aed527671a218a8e0a5b2a9e541578fbd775176981f22df31fddd6e.",
            ),
            exact_function_evidence(
                "binary.gz2e01.memory-to-card",
                "5b65a8833c8fb246e5c0292e0f22ecf6b05f5e3a123f2f18ee33c343a9805f1e",
                "memory_to_card__10dSv_info_cFPci at VA 0x80035798, size 0x26c, code SHA-256 7cf6fc958ed1e4cdcf4b3e168364cbd7a42a545a1812d139a4442e41ae5fd8e9.",
            ),
            exact_function_evidence(
                "binary.gz2e01.save-menu-data-write",
                "cf1308d2ecb1741549ce173a76f7e7c0ff8fe7343156632baae499dea1836ebb",
                "dataWrite__12dMenu_save_cFv at VA 0x801f2840, size 0xa4, code SHA-256 b6a30e6925392a2c876f0f002e93afeb257da6878b989515c12fe83b58c6ac35.",
            ),
            exact_function_evidence(
                "binary.gz2e01.save-menu-wait",
                "8b8f2e635426fdd8dc3e4cf4c49953ef1518e6836dca669acbd5cd5706ad0394",
                "memCardDataSaveWait__12dMenu_save_cFv at VA 0x801f28e4, size 0xa8, code SHA-256 ab833e5d0f988b09921e3788272ebaa325767f91f649af3209ff0bcff6b40778.",
            ),
            exact_function_evidence(
                "binary.gz2e01.save-menu-wait-2",
                "c0bdf0610b4b25b22ddf5dab9745bbf8dfdd8267d02daaf878186335eb3b1d88",
                "memCardDataSaveWait2__12dMenu_save_cFv at VA 0x801f298c, size 0x1d0, code SHA-256 206affd3eccd29c55beed5853501307985d355504ab3c4d5ebbb076dd719022f.",
            ),
        ],
    };
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
            component_id: DUNGEON_SIX_SAVE_COMPONENT.into(),
            payload: ComponentPayload::Structured {
                fields: BTreeMap::from([("key_count".into(), StateValue::Unsigned(0))]),
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
            component_id: PLAYER_INFO_COMPONENT.into(),
            payload: ComponentPayload::Structured {
                fields: BTreeMap::from([
                    (
                        "horse_name_bytes".into(),
                        StateValue::Bytes(DEFAULT_HORSE_NAME_BYTES.to_vec()),
                    ),
                    (
                        "player_name_bytes".into(),
                        StateValue::Bytes(DEFAULT_PLAYER_NAME_BYTES.to_vec()),
                    ),
                    ("total_time_ticks".into(), StateValue::Unsigned(0)),
                    ("date_ipl_ticks".into(), StateValue::Unsigned(0)),
                ]),
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
            ("maximum_oil".into(), StateValue::Unsigned(0)),
            ("oil".into(), StateValue::Unsigned(0)),
            ("inventory".into(), StateValue::Bytes(vec![0xff; 24])),
            ("item_lineup".into(), StateValue::Bytes(vec![0xff; 24])),
            ("selected_items".into(), StateValue::Bytes(vec![0xff; 4])),
            ("mixed_items".into(), StateValue::Bytes(vec![0xff; 4])),
            ("vibration".into(), StateValue::Unsigned(1)),
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

fn file_select_post_copy_normalization() -> Vec<StateOperation> {
    vec![
        StateOperation::ClampUnsignedMinimum {
            target: ComponentFieldTarget {
                component_id: INVENTORY_COMPONENT.into(),
                field: "life".into(),
            },
            minimum: 12,
        },
        StateOperation::Write {
            target: ComponentFieldTarget {
                component_id: DUNGEON_SIX_SAVE_COMPONENT.into(),
                field: "key_count".into(),
            },
            value: StateValue::Unsigned(0),
        },
        StateOperation::NormalizeItemSlotsAndLineup {
            component_id: INVENTORY_COMPONENT.into(),
            inventory_field: "inventory".into(),
            lineup_field: "item_lineup".into(),
            primary_slot: 9,
            secondary_slot: 10,
            single_item: ITEM_HOOKSHOT,
            combined_item: ITEM_DOUBLE_CLAWSHOT,
            empty_item: ITEM_NONE,
            lineup_order: ITEM_LINEUP_ORDER.to_vec(),
        },
        StateOperation::CopyValue {
            source: ComponentFieldTarget {
                component_id: INVENTORY_COMPONENT.into(),
                field: "vibration".into(),
            },
            target: ComponentFieldTarget {
                component_id: ACTIVE_VIBRATION_COMPONENT.into(),
                field: "enabled_raw".into(),
            },
        },
        StateOperation::CopyValue {
            source: ComponentFieldTarget {
                component_id: RETURN_PLACE_COMPONENT.into(),
                field: "stage".into(),
            },
            target: ComponentFieldTarget {
                component_id: SAVE_STAGE_DISPLAY_COMPONENT.into(),
                field: "stage".into(),
            },
        },
    ]
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
            DUNGEON_SIX_SAVE_COMPONENT,
            ComponentKind::DungeonMemory,
            ComponentPayload::Structured {
                fields: BTreeMap::from([("key_count".into(), StateValue::Unsigned(0))]),
            },
        ),
        component(
            PLAYER_INFO_COMPONENT,
            ComponentKind::Custom {
                id: "player-info".into(),
            },
            ComponentPayload::Structured {
                fields: BTreeMap::from([
                    ("horse_name_bytes".into(), StateValue::Bytes(vec![0])),
                    ("player_name_bytes".into(), StateValue::Bytes(vec![0])),
                    ("total_time_ticks".into(), StateValue::Unsigned(0)),
                    ("date_ipl_ticks".into(), StateValue::Unsigned(0)),
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

    fn fields_for<'a>(
        state: &'a PlannerExecutionState,
        id: &str,
    ) -> &'a BTreeMap<String, StateValue> {
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
        let StateValue::Bytes(items) = &fields_for(&existing, INVENTORY_COMPONENT)["inventory"]
        else {
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
}
