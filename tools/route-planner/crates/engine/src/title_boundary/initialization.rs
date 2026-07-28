//! Construct initialized save-domain payloads and file-select buffers.

use super::*;

pub(super) fn parse_digest(value: &str) -> Digest {
    value.parse().expect("compile-time SHA-256 literal")
}

pub(super) fn exact_function_evidence(
    id: &str,
    artifact_sha256: &str,
    note: &str,
) -> EvidenceRecord {
    EvidenceRecord {
        id: id.into(),
        kind: EvidenceKind::Extracted,
        source_sha256: Some(parse_digest(artifact_sha256)),
        note: note.into(),
    }
}

pub(super) fn dcomifgs_init_effects() -> Vec<StateOperation> {
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

pub(super) fn base_inventory_payload() -> ComponentPayload {
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

pub(super) fn file_select_post_copy_normalization() -> Vec<StateOperation> {
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

pub(super) fn file_select_buffer_owner(slot: u8) -> SerializationOwner {
    SerializationOwner::Custom {
        id: format!("{FILE_SELECT_BUFFER_OWNER_PREFIX}-{slot}"),
    }
}

pub(super) fn initialized_file_select_buffer(slot: u8) -> Vec<StateComponent> {
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
