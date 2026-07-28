use super::*;
use crate::identity::{
    CONTENT_IDENTITY_SCHEMA, ContentFingerprint, GamePlatform, GameRegion,
    RUNTIME_CONFIGURATION_SCHEMA,
};
use crate::orig_discovery::bundled_supported_build_registry;
use crate::world_data::{
    CollisionCode, CollisionInventoryRecord, CollisionLoadTrigger, KclAuthoredPrism,
    KclInventoryPrism, KclReconstruction, KclSourceIndices, PlacementKind, PlacementRecord,
    SourceKind, SourceScope, StageExitRecord, Vec3, WORLD_INVENTORY_SCHEMA, WorldContext,
    WorldInventory, WorldSource,
};

fn content() -> ContentIdentity {
    ContentIdentity {
        schema: CONTENT_IDENTITY_SCHEMA.into(),
        id: "gcn-us-test".into(),
        fingerprint: ContentFingerprint {
            platform: GamePlatform::GameCube,
            region: GameRegion::Usa,
            revision: "test".into(),
            product_id: "GZ2E01".into(),
            executable_sha256: Digest([1; 32]),
            game_data_sha256: Digest([2; 32]),
            resource_manifest_sha256: Digest([3; 32]),
        },
    }
}

fn audited_content() -> ContentIdentity {
    bundled_supported_build_registry()
        .unwrap()
        .identities
        .into_iter()
        .find(|identity| identity.id == "gcn-us-1.0-gz2e01")
        .unwrap()
}

fn runtime(content: &ContentIdentity) -> RuntimeConfiguration {
    RuntimeConfiguration {
        schema: RUNTIME_CONFIGURATION_SCHEMA.into(),
        content_sha256: content.digest().unwrap(),
        language: "en".into(),
        settings: BTreeMap::new(),
    }
}

fn placement(stable_id: &str, kind: PlacementKind, name: &str, angle_z: i16) -> PlacementRecord {
    PlacementRecord {
        stable_id: stable_id.into(),
        source_sha256: Digest([12; 32]),
        scope: SourceScope {
            kind: SourceKind::Room,
            room: Some(0),
        },
        chunk_tag: if kind == PlacementKind::PlayerSpawn {
            "PLYR".into()
        } else {
            "ACTR".into()
        },
        record_index: 0,
        layer: None,
        kind,
        name: name.into(),
        parameters: 0x1234,
        position: Vec3 {
            x: 10.0,
            y: 20.0,
            z: 30.0,
        },
        angle: [0, 0x1000, angle_z],
        set_id: 7,
        scale_raw: None,
        raw_hex: "00".repeat(32),
    }
}

fn inventory(linked: bool) -> WorldInventory {
    let kcl = Digest([14; 32]);
    let plc = Digest([15; 32]);
    let collision_id = format!("kcl-sha256:{kcl}/plc-sha256:{plc}/prism/1");
    let exit_id = "dzr-sha256:fixture/chunk/SCLS/record/0".to_string();
    let raw_code = [if linked { 0 } else { 0x3f }, 0, 0, 0, 0];
    let collision = CollisionInventoryRecord {
        room: 0,
        prism: KclInventoryPrism {
            authored: KclAuthoredPrism {
                stable_id: collision_id.clone(),
                prism_index: 1,
                height: 1.0,
                source_indices: KclSourceIndices {
                    position: 0,
                    face_normal: 0,
                    edge_normal_1: 0,
                    edge_normal_2: 0,
                    edge_normal_3: 0,
                },
                attribute: 0,
                code: CollisionCode {
                    raw: raw_code,
                    exit_id: if linked { 0 } else { 0x3f },
                    polygon_color: 0,
                    special_code: 0,
                    link_no: 0,
                    wall_code: 0,
                    attribute_0: 0,
                    attribute_1: 0,
                    ground_code: 0,
                    camera_move_background: 0,
                    room_camera: 0,
                    room_path: 0,
                    room_path_point: 0,
                    room_info: 0,
                    sound_id: 0,
                    room: 0,
                },
            },
            reconstruction: KclReconstruction::Degenerate {
                reason: "fixture".into(),
            },
        },
    };
    let exit = StageExitRecord {
        stable_id: exit_id.clone(),
        source_sha256: Digest([12; 32]),
        scope: SourceScope {
            kind: SourceKind::Room,
            room: Some(0),
        },
        chunk_tag: "SCLS".into(),
        record_index: 0,
        destination_stage: "F_SP104".into(),
        destination_point: 2,
        destination_room: 1,
        destination_layer: -1,
        wipe: 0,
        wipe_time: 0,
        time_hour: -1,
        raw_start: 2,
        raw_field_a: 0,
        raw_field_b: 0x0f,
        raw_wipe: 15,
        raw_hex: "00".repeat(13),
    };
    let load_triggers = if linked {
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight.collision-load-trigger/v1\0");
        hasher.update(collision_id.as_bytes());
        hasher.update([0]);
        hasher.update(exit_id.as_bytes());
        let digest = Digest(hasher.finalize().into());
        vec![CollisionLoadTrigger {
            stable_id: format!("load-trigger-sha256:{digest}"),
            room: 0,
            collision_id,
            collision_exit_id: 0,
            scls_id: exit_id,
            destination_stage: "F_SP104".into(),
            destination_room: 1,
            destination_layer: -1,
            destination_point: 2,
            inferred_semantics: true,
        }]
    } else {
        Vec::new()
    };
    WorldInventory {
        schema: WORLD_INVENTORY_SCHEMA.into(),
        stage: "F_SP103".into(),
        sources: vec![
            WorldSource {
                scope: SourceScope {
                    kind: SourceKind::Stage,
                    room: None,
                },
                archive_sha256: Digest([10; 32]),
                stage_data_path: "stage.dzs".into(),
                stage_data_sha256: Digest([11; 32]),
                kcl_path: None,
                kcl_sha256: None,
                plc_path: None,
                plc_sha256: None,
                addressable_prisms: 0,
            },
            WorldSource {
                scope: SourceScope {
                    kind: SourceKind::Room,
                    room: Some(0),
                },
                archive_sha256: Digest([13; 32]),
                stage_data_path: "room.dzr".into(),
                stage_data_sha256: Digest([12; 32]),
                kcl_path: Some("room.kcl".into()),
                kcl_sha256: Some(kcl),
                plc_path: Some("room.plc".into()),
                plc_sha256: Some(plc),
                addressable_prisms: 1,
            },
        ],
        chunks: Vec::new(),
        placements: vec![placement(
            "actor-record",
            PlacementKind::Actor,
            "kytag14",
            0,
        )],
        player_spawns: vec![placement(
            "spawn-record",
            PlacementKind::PlayerSpawn,
            "start",
            5,
        )],
        exits: vec![exit],
        collisions: vec![collision],
        load_triggers,
    }
}

fn boss_door_inventory(room: i8) -> WorldInventory {
    let room_resource =
        static_digest("9336aabaee513b635d6d0d3db3f5f3b67f5c6bd6643581ebd1a8f7b779fa8e7a");
    let exit_id = "dzr-sha256:boss-room/chunk/SCLS/record/0".to_string();
    WorldInventory {
        schema: WORLD_INVENTORY_SCHEMA.into(),
        stage: "D_MN05".into(),
        sources: vec![
            WorldSource {
                scope: SourceScope {
                    kind: SourceKind::Stage,
                    room: None,
                },
                archive_sha256: Digest([10; 32]),
                stage_data_path: "stage.dzs".into(),
                stage_data_sha256: Digest([11; 32]),
                kcl_path: None,
                kcl_sha256: None,
                plc_path: None,
                plc_sha256: None,
                addressable_prisms: 0,
            },
            WorldSource {
                scope: SourceScope {
                    kind: SourceKind::Room,
                    room: Some(room),
                },
                archive_sha256: static_digest(
                    "5b495a915c1539b92f57e84f7cbcf0b5662a8caeaf7ecf0503ac15af7a6e6a77",
                ),
                stage_data_path: "room.dzr".into(),
                stage_data_sha256: room_resource,
                kcl_path: None,
                kcl_sha256: None,
                plc_path: None,
                plc_sha256: None,
                addressable_prisms: 0,
            },
        ],
        chunks: Vec::new(),
        placements: vec![PlacementRecord {
            stable_id: "dzr-sha256:boss-room/chunk/ACTR/record/0".into(),
            source_sha256: room_resource,
            scope: SourceScope {
                kind: SourceKind::Room,
                room: Some(room),
            },
            chunk_tag: "ACTR".into(),
            record_index: 0,
            layer: None,
            kind: PlacementKind::ScaledActor,
            name: "L1Bdoor".into(),
            parameters: 0x0191_8000,
            position: Vec3 {
                x: 7283.0,
                y: 3302.0,
                z: -16430.0,
            },
            angle: [-211, 0, 0x1717],
            set_id: 0xff,
            scale_raw: Some([10, 10, 10]),
            raw_hex: "4c3142646f6f72000191800045e39800454e6000c6805c00ff2d0000171700ff0a0a0aff"
                .into(),
        }],
        player_spawns: Vec::new(),
        exits: vec![StageExitRecord {
            stable_id: exit_id,
            source_sha256: room_resource,
            scope: SourceScope {
                kind: SourceKind::Room,
                room: Some(room),
            },
            chunk_tag: "SCLS".into(),
            record_index: 0,
            destination_stage: "D_MN05A".into(),
            destination_point: 0,
            destination_room: 50,
            destination_layer: -1,
            wipe: 0,
            wipe_time: 0,
            time_hour: -1,
            raw_start: 0,
            raw_field_a: 0,
            raw_field_b: 0,
            raw_wipe: 0,
            raw_hex: "00".repeat(13),
        }],
        collisions: Vec::new(),
        load_triggers: Vec::new(),
    }
}

fn l5_boss_door_inventory(boss_room: bool) -> WorldInventory {
    let room = if boss_room { 50 } else { 4 };
    let mut inventory = boss_door_inventory(room);
    inventory.stage = if boss_room { "D_MN11A" } else { "D_MN11" }.into();
    let (archive_sha256, resource_sha256, parameters, position, destination) = if boss_room {
        (
            static_digest("4acd3b8ce5ac24820364314c1cbec9569bf0faad2d4f0e6688e974616d8c7889"),
            static_digest("106533086f77371b6abd4cfea2d0d2c14fd88f5ce1a2569bfc8020848d2519a6"),
            0x0390_8200,
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: 2100.0,
            },
            ("F_SP114", 1, 2, 11, 1),
        )
    } else {
        (
            static_digest("6ee1274731222f3abe62c50de686fbae60663ba11e911f79c54fe886e221cd55"),
            static_digest("cd32b1ac737b8cfe6f92fa35a18ba4e24a5ccbfe8b466ae403b6c269dcbfc5c3"),
            0x0590_8200,
            Vec3 {
                x: 0.0,
                y: 2109.0,
                z: -5237.0,
            },
            ("D_MN11A", 50, -1, 1, 2),
        )
    };
    inventory.sources[1].archive_sha256 = archive_sha256;
    inventory.sources[1].stage_data_sha256 = resource_sha256;
    let placement = &mut inventory.placements[0];
    placement.stable_id = format!("dzr-sha256:l5-{room}/chunk/Door/record/0");
    placement.source_sha256 = resource_sha256;
    placement.chunk_tag = "Door".into();
    placement.name = "L5Bdoor".into();
    placement.parameters = parameters;
    placement.position = position;
    placement.angle = [-1, 0, -248];
    placement.raw_hex = if boss_room {
        "4c3542646f6f720003908200000000000000000045034000ffff0000ff0800ff0a0a0aff"
    } else {
        "4c3542646f6f720005908200000000004503d000c5a3a800ffff0000ff0800ff0a0a0aff"
    }
    .into();
    let exit = &mut inventory.exits[0];
    exit.stable_id = format!("dzr-sha256:l5-{room}/chunk/SCLS/record/{}", destination.4);
    exit.source_sha256 = resource_sha256;
    exit.record_index = destination.4;
    exit.destination_stage = destination.0.into();
    exit.destination_room = destination.1;
    exit.destination_layer = destination.2;
    exit.destination_point = destination.3;
    inventory
}

fn replace_room_actor(
    mut inventory: WorldInventory,
    stage: &str,
    room: i8,
    placement: PlacementRecord,
    keep_exit: bool,
) -> WorldInventory {
    inventory.stage = stage.into();
    inventory.sources[1].scope.room = Some(room);
    inventory.sources[1].stage_data_sha256 = placement.source_sha256;
    inventory.placements = vec![placement];
    if keep_exit {
        inventory.exits[0].scope.room = Some(room);
        inventory.exits[0].source_sha256 = inventory.sources[1].stage_data_sha256;
    } else {
        inventory.exits.clear();
    }
    inventory
}

fn keyed_mboss_inventory() -> WorldInventory {
    let source = static_digest("2756f041cd797b24e2794983d3c6e0b370aa1dd50a57a08a2e437585688a268c");
    let mut inventory = replace_room_actor(
        boss_door_inventory(7),
        "D_MN06",
        7,
        PlacementRecord {
            stable_id: format!("dzr-sha256:{source}/chunk/Door/record/0"),
            source_sha256: source,
            scope: SourceScope {
                kind: SourceKind::Room,
                room: Some(7),
            },
            chunk_tag: "Door".into(),
            record_index: 0,
            layer: None,
            kind: PlacementKind::ScaledActor,
            name: "L6Mdoor".into(),
            parameters: 0x01b0_e600,
            position: Vec3 {
                x: 1580.0,
                y: 8250.0,
                z: 700.0,
            },
            angle: [-1, 0x4000, -227],
            set_id: 0xff,
            scale_raw: Some([10, 10, 10]),
            raw_hex: "4c364d646f6f720001b0e60044c580004600e800442f0000ffff4000ff1d00ff0a0a0aff"
                .into(),
        },
        true,
    );
    let exit = &mut inventory.exits[0];
    exit.stable_id = format!("dzr-sha256:{source}/chunk/SCLS/record/0");
    exit.destination_stage = "D_MN06B".into();
    exit.destination_room = 51;
    exit.destination_layer = -1;
    exit.destination_point = 0;
    inventory
}

fn regular_key_shutter_inventory() -> WorldInventory {
    let source = static_digest("f4a05b52105afd1dacb6b5b4b8e51706a922be24ea1b15fc198b47fd8aefb578");
    replace_room_actor(
        boss_door_inventory(9),
        "D_MN01",
        9,
        PlacementRecord {
            stable_id: format!("dzr-sha256:{source}/chunk/ACTR/record/4"),
            source_sha256: source,
            scope: SourceScope {
                kind: SourceKind::Room,
                room: Some(9),
            },
            chunk_tag: "ACTR".into(),
            record_index: 4,
            layer: None,
            kind: PlacementKind::Actor,
            name: "kshtr00".into(),
            parameters: 0x80ff_0123,
            position: Vec3 {
                x: 15185.0,
                y: -50.0,
                z: -570.0,
            },
            angle: [0, -5461, 255],
            set_id: 0xffff,
            scale_raw: None,
            raw_hex: "6b7368747230300080ff0123466d4400c2480000c40e80000000eaab00ffffff".into(),
        },
        false,
    )
}

fn lakebed_boss_key_shutter_inventory() -> WorldInventory {
    let source = static_digest("9336aabaee513b635d6d0d3db3f5f3b67f5c6bd6643581ebd1a8f7b779fa8e7a");
    replace_room_actor(
        boss_door_inventory(3),
        "D_MN01",
        3,
        PlacementRecord {
            stable_id: format!("dzr-sha256:{source}/chunk/Door/record/0"),
            source_sha256: source,
            scope: SourceScope {
                kind: SourceKind::Room,
                room: Some(3),
            },
            chunk_tag: "Door".into(),
            record_index: 0,
            layer: None,
            kind: PlacementKind::ScaledActor,
            name: "L3Bdoor".into(),
            parameters: 0x80ff_0255,
            position: Vec3 {
                x: 0.0,
                y: -320.0,
                z: 325.31067,
            },
            angle: [0, 0, 0],
            set_id: 0xff,
            scale_raw: Some([10, 10, 10]),
            raw_hex: "4c3342646f6f720080ff025500000000c3a0000043a2a7c400000000000000ff0a0a0aff"
                .into(),
        },
        false,
    )
}

fn koki_gate_inventory(switch_id: u8) -> WorldInventory {
    let source = static_digest("5c2208b4088c8ac55dabca200f7bd7eedac3cf2c93364eb49fcfae2216513e21");
    let parameters = 0x0ff0_ff00 | u32::from(switch_id);
    let mut raw = "4b5f4761746500000ff0ff0cc688e00043480000c62eec00000026660000ffff".to_owned();
    raw.replace_range(22..24, &format!("{switch_id:02x}"));
    replace_room_actor(
        boss_door_inventory(3),
        "F_SP108",
        3,
        PlacementRecord {
            stable_id: format!("dzr-sha256:{source}/chunk/ACT0/record/36"),
            source_sha256: source,
            scope: SourceScope {
                kind: SourceKind::Room,
                room: Some(3),
            },
            chunk_tag: "ACT0".into(),
            record_index: 36,
            layer: Some(0),
            kind: PlacementKind::Actor,
            name: "K_Gate".into(),
            parameters,
            position: Vec3 {
                x: -17520.0,
                y: 200.0,
                z: -11195.0,
            },
            angle: [0, 9830, 0],
            set_id: 0xffff,
            scale_raw: None,
            raw_hex: raw,
        },
        false,
    )
}

fn rider_gate_inventory() -> WorldInventory {
    let source = static_digest("22482b0344bcbb4a068562e088684590a3c27190ebfcc2d5c41a9a51c7b109f6");
    replace_room_actor(
        boss_door_inventory(0),
        "F_SP109",
        0,
        PlacementRecord {
            stable_id: format!("dzr-sha256:{source}/chunk/ACT0/record/10"),
            source_sha256: source,
            scope: SourceScope {
                kind: SourceKind::Room,
                room: Some(0),
            },
            chunk_tag: "ACT0".into(),
            record_index: 10,
            layer: Some(0),
            kind: PlacementKind::Actor,
            name: "R_Gate".into(),
            parameters: 0x0ff2_ff6b,
            position: Vec3 {
                x: -8055.0,
                y: 780.0,
                z: -8235.0,
            },
            angle: [0, -16384, 0],
            set_id: 0xffff,
            scale_raw: None,
            raw_hex: "525f4761746500000ff2ff6bc5fbb80044430000c600ac000000c0000000ffff".into(),
        },
        false,
    )
}

fn caravan_gate_inventory() -> WorldInventory {
    let source = static_digest("1f60355fcaab8b2b0c4d32b62ac638049952d36f5fb3bc7a81472708402639a4");
    replace_room_actor(
        boss_door_inventory(1),
        "F_SP118",
        1,
        PlacementRecord {
            stable_id: format!("dzr-sha256:{source}/chunk/ACT0/record/23"),
            source_sha256: source,
            scope: SourceScope {
                kind: SourceKind::Room,
                room: Some(1),
            },
            chunk_tag: "ACT0".into(),
            record_index: 23,
            layer: Some(0),
            kind: PlacementKind::Actor,
            name: "CrvGate".into(),
            parameters: u32::MAX,
            position: Vec3 {
                x: 2150.0,
                y: 0.0,
                z: -450.0,
            },
            angle: [0, -32768, 0],
            set_id: 0xffff,
            scale_raw: None,
            raw_hex: "4372764761746500ffffffff4506600000000000c3e10000000080000000ffff".into(),
        },
        false,
    )
}

fn l7_bridge_demo_inventory() -> WorldInventory {
    let source = static_digest("a7014eb0a33bb9a57af75caff72605f6725273909f5bd2cf61c465c140fe6a6e");
    let placement = PlacementRecord {
        stable_id: format!("dzr-sha256:{source}/chunk/ACTR/record/15"),
        source_sha256: source,
        scope: SourceScope {
            kind: SourceKind::Room,
            room: Some(6),
        },
        chunk_tag: "ACTR".into(),
        record_index: 15,
        layer: None,
        kind: PlacementKind::Actor,
        name: "dr".into(),
        parameters: 0x18,
        position: Vec3 {
            x: -7075.0,
            y: -200.0,
            z: -11809.403,
        },
        angle: [0, -32768, 0],
        set_id: 0xffff,
        scale_raw: None,
        raw_hex: "647200000000000000000018c5dd1800c3480000c638859d000080000000ffff".into(),
    };
    let mut inventory = replace_room_actor(boss_door_inventory(6), "D_MN07", 6, placement, false);
    inventory.exits = [
        (6, 7, 3, "445f4d4e303700000706f03301"),
        (7, 8, -1, "445f4d4e303700000806f03f00"),
    ]
    .into_iter()
    .map(|(record_index, spawn, layer, raw_hex)| StageExitRecord {
        stable_id: format!("dzr-sha256:{source}/chunk/SCLS/record/{record_index}"),
        source_sha256: source,
        scope: SourceScope {
            kind: SourceKind::Room,
            room: Some(6),
        },
        chunk_tag: "SCLS".into(),
        record_index,
        destination_stage: "D_MN07".into(),
        destination_point: spawn,
        destination_room: 6,
        destination_layer: layer,
        wipe: if record_index == 6 { 1 } else { 0 },
        wipe_time: 1,
        time_hour: -1,
        raw_start: spawn as u8,
        raw_field_a: 0xf0,
        raw_field_b: if record_index == 6 { 0x33 } else { 0x3f },
        raw_wipe: if record_index == 6 { 1 } else { 0 },
        raw_hex: raw_hex.into(),
    })
    .collect();
    inventory
}

fn has_small_key_comparison(
    transition: &CandidateTransition,
    expected_operator: ComparisonOperator,
    expected_value: u64,
) -> bool {
    let PredicateExpression::All { terms } = &transition.activation.hard_guards else {
        return false;
    };
    terms.iter().any(|term| {
        matches!(
            term,
            PredicateExpression::Compare {
                left: ValueReference::BoundRawBits {
                    byte_offset: 0x1c,
                    byte_width: 1,
                    mask: 0xff,
                    ..
                },
                operator,
                right: ValueReference::Literal {
                    value: StateValue::Unsigned(value),
                },
            } if *operator == expected_operator && *value == expected_value
        )
    })
}

fn predicate_has_layer_comparison(
    predicate: &PredicateExpression,
    expected_operator: ComparisonOperator,
    expected_layer: i64,
) -> bool {
    match predicate {
        PredicateExpression::Compare {
            left: ValueReference::LocationLayer,
            operator,
            right:
                ValueReference::Literal {
                    value: StateValue::Signed(layer),
                },
        } => *operator == expected_operator && *layer == expected_layer,
        PredicateExpression::All { terms } | PredicateExpression::Any { terms } => terms
            .iter()
            .any(|term| predicate_has_layer_comparison(term, expected_operator, expected_layer)),
        PredicateExpression::Not { term } => {
            predicate_has_layer_comparison(term, expected_operator, expected_layer)
        }
        _ => false,
    }
}

fn predicate_has_persistent_event_bit(predicate: &PredicateExpression, set: bool) -> bool {
    match predicate {
        PredicateExpression::Compare {
            left:
                ValueReference::BoundRawBits {
                    component_kind: ComponentKind::Custom { id },
                    binding: ComponentBindingReference::ActiveRuntimeFile,
                    byte_offset: 0x08,
                    byte_width: 1,
                    mask: 0x10,
                },
            operator: ComparisonOperator::Equal,
            right:
                ValueReference::Literal {
                    value: StateValue::Unsigned(value),
                },
        } => id == "persistent-event-registers" && *value == if set { 0x10 } else { 0 },
        PredicateExpression::All { terms } | PredicateExpression::Any { terms } => terms
            .iter()
            .any(|term| predicate_has_persistent_event_bit(term, set)),
        PredicateExpression::Not { term } => predicate_has_persistent_event_bit(term, set),
        _ => false,
    }
}

fn writes_small_key(transition: &CandidateTransition, expected_value: u8) -> bool {
    transition.activation.effects.iter().any(|effect| {
        matches!(
            effect,
            StateOperation::WriteBoundRaw {
                component_kind: ComponentKind::DungeonMemory,
                binding: ComponentBindingReference::CurrentStage,
                byte_offset: 0x1c,
                mask,
                value,
            } if mask == &[0xff] && value == &[expected_value]
        )
    })
}

fn world_context(game_data_sha256: Digest, inventory: &WorldInventory) -> WorldContext {
    let context = WorldContext {
        schema: crate::world_data::WORLD_CONTEXT_SCHEMA.into(),
        game_data_sha256,
        stages: vec![crate::world_data::WorldContextStage {
            stage: inventory.stage.clone(),
            inventory_sha256: inventory.digest().unwrap(),
            spatial_index_sha256: Digest([99; 32]),
        }],
    };
    context.validate().unwrap();
    context
}

#[test]
fn imports_joined_exit_as_obstructed_candidate_without_claiming_feasibility() {
    let content = content();
    let runtime = runtime(&content);
    let inventory = inventory(true);
    inventory.validate().unwrap();
    let context = world_context(Digest([2; 32]), &inventory);
    let facts = ExtractedWorldFacts::build(
        &content,
        &runtime,
        &context,
        std::slice::from_ref(&inventory),
    )
    .unwrap();
    assert_eq!(facts.static_world_objects.len(), 2);
    assert_eq!(facts.spawns[0].location.spawn, 5);
    assert_eq!(facts.mechanics.transitions.len(), 1);
    assert_eq!(facts.mechanics.obligations.len(), 1);
    assert_eq!(facts.approach_geometries.len(), 1);
    assert!(matches!(
        facts.approach_geometries[0].shape,
        ExtractedApproachShape::Unavailable { .. }
    ));
    assert_eq!(
        facts.approach_geometries[0].candidate_spawn_ids,
        vec![facts.spawns[0].id.clone()]
    );
    assert_eq!(facts.encoded_exits[0].candidate_transition_ids.len(), 1);
    let transition = &facts.mechanics.transitions[0];
    assert_eq!(transition.evidence.truth, TruthStatus::Contested);
    assert_eq!(transition.activation.unknown_requirements.len(), 1);
    assert_eq!(transition.activation.physical_obligation_ids.len(), 1);
    assert!(matches!(
        transition.activation.effects.as_slice(),
        [StateOperation::SetLocation { location }]
            if location.stage == "F_SP104" && location.room == 1 && location.spawn == 2
    ));
    let bytes = facts.canonical_bytes().unwrap();
    assert_eq!(
        ExtractedWorldFacts::decode_canonical(&bytes).unwrap(),
        facts
    );
    assert_ne!(facts.digest().unwrap(), Digest::ZERO);
}

#[test]
fn derives_collision_triangle_bounds_and_same_room_spawn_candidates() {
    let content = content();
    let runtime = runtime(&content);
    let mut inventory = inventory(true);
    inventory.collisions[0].prism.reconstruction = KclReconstruction::Reconstructed {
        plane: crate::world_data::CollisionPlane {
            anchor: Vec3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            normal: Vec3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
            d: 0.0,
        },
        triangle: [
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            Vec3 {
                x: 2.0,
                y: 0.0,
                z: 0.0,
            },
            Vec3 {
                x: 0.0,
                y: 3.0,
                z: 0.0,
            },
        ],
    };
    inventory.validate().unwrap();
    let context = world_context(Digest([2; 32]), &inventory);
    let facts = ExtractedWorldFacts::build(
        &content,
        &runtime,
        &context,
        std::slice::from_ref(&inventory),
    )
    .unwrap();
    let geometry = &facts.approach_geometries[0];
    assert_eq!(geometry.transition_id, facts.mechanics.transitions[0].id);
    assert_eq!(
        geometry.approach_id,
        facts.mechanics.transitions[0].approach_id
    );
    assert_eq!(
        geometry.candidate_spawn_ids,
        vec![facts.spawns[0].id.clone()]
    );
    assert!(matches!(
        geometry.shape,
        ExtractedApproachShape::Reconstructed {
            minimum: [0.0, 0.0, 0.0],
            maximum: [2.0, 3.0, 0.0],
            plane_normal: [0.0, 1.0, 0.0],
            plane_offset: 0.0,
            ..
        }
    ));

    let mut tampered = facts;
    let ExtractedApproachShape::Reconstructed { maximum, .. } =
        &mut tampered.approach_geometries[0].shape
    else {
        unreachable!();
    };
    maximum[0] = 3.0;
    assert_eq!(
        tampered.validate().unwrap_err().field(),
        "approach_geometries.shape"
    );
}

#[test]
fn imports_audited_gz2e01_boss_door_guard_write_and_destination() {
    assert_eq!(memory_switch_raw_location(0x00), (0x0b, 0x01));
    assert_eq!(memory_switch_raw_location(0x17), (0x09, 0x80));
    assert_eq!(memory_switch_raw_location(0x1f), (0x08, 0x80));
    assert_eq!(memory_switch_raw_location(0x20), (0x0f, 0x01));
    assert_eq!(memory_switch_raw_location(0x7f), (0x14, 0x80));
    let content = audited_content();
    let runtime = runtime(&content);
    let inventory = boss_door_inventory(12);
    inventory.validate().unwrap();
    let context = world_context(content.fingerprint.game_data_sha256, &inventory);
    let facts = ExtractedWorldFacts::build(
        &content,
        &runtime,
        &context,
        std::slice::from_ref(&inventory),
    )
    .unwrap();

    assert_eq!(facts.mechanics.transitions.len(), 1);
    assert_eq!(facts.mechanics.obligations.len(), 3);
    assert_eq!(facts.spatial_volumes.len(), 2);
    assert_eq!(facts.encoded_exits[0].candidate_transition_ids.len(), 1);
    let transition = &facts.mechanics.transitions[0];
    assert_eq!(transition.transition_kind, TransitionKind::Door);
    assert_eq!(transition.evidence.truth, TruthStatus::Established);
    assert_eq!(transition.activation.physical_obligation_ids.len(), 3);
    assert!(facts.mechanics.obligations.iter().any(|obligation| {
        matches!(
            &obligation.detail,
            ObligationDetail::CompoundInteraction { branches, .. }
                if branches.len() == 2
                    && branches[1].volume_tests.iter().any(|test| {
                        test.position == crate::transition::InteractionPosition::PlayerAttention
                    })
                    && branches[1].volume_tests.iter().any(|test| {
                        test.position == crate::transition::InteractionPosition::Player
                            && test.volume.volume_id == "boss-door-wolf-current-x"
                    })
        )
    }));
    assert!(matches!(
        facts.spatial_volumes[1].shape,
        SpatialVolumeShape::YawOrientedStrip {
            axis: crate::state::SpatialLocalAxis::X,
            minimum: -130.0,
            maximum: 130.0,
            ..
        }
    ));
    let PredicateExpression::All { terms } = &transition.activation.hard_guards else {
        panic!("boss door must retain source location and boss-key guards")
    };
    assert!(terms.iter().any(|term| {
        matches!(
            term,
            PredicateExpression::Compare {
                left: ValueReference::BoundRawBits {
                    component_kind: ComponentKind::DungeonMemory,
                    binding: ComponentBindingReference::CurrentStage,
                    byte_offset: 0x1d,
                    byte_width: 1,
                    mask: 0x04,
                },
                operator: ComparisonOperator::Equal,
                right: ValueReference::Literal {
                    value: StateValue::Unsigned(0x04),
                },
            }
        )
    }));
    assert!(matches!(
        transition.activation.effects.as_slice(),
        [
            StateOperation::WriteBoundRaw {
                component_kind: ComponentKind::DungeonMemory,
                binding: ComponentBindingReference::CurrentStage,
                byte_offset: 0x09,
                mask,
                value,
            },
            StateOperation::SetLocation { location },
        ] if mask == &[0x80]
            && value == &[0x80]
            && location.stage == "D_MN05A"
            && location.room == 50
            && location.spawn == 0
    ));
}

#[test]
fn does_not_generalize_boss_door_source_semantics_or_reverse_side() {
    let inventory = boss_door_inventory(12);
    let mut unaudited_content = audited_content();
    unaudited_content.fingerprint.executable_sha256 = Digest([0x55; 32]);
    let unaudited_runtime = runtime(&unaudited_content);
    let context = world_context(unaudited_content.fingerprint.game_data_sha256, &inventory);
    let facts = ExtractedWorldFacts::build(
        &unaudited_content,
        &unaudited_runtime,
        &context,
        std::slice::from_ref(&inventory),
    )
    .unwrap();
    assert!(facts.mechanics.transitions.is_empty());
    assert!(facts.encoded_exits[0].candidate_transition_ids.is_empty());

    let content = audited_content();
    let runtime = runtime(&content);
    let reverse_inventory = boss_door_inventory(50);
    let context = world_context(content.fingerprint.game_data_sha256, &reverse_inventory);
    let facts = ExtractedWorldFacts::build(
        &content,
        &runtime,
        &context,
        std::slice::from_ref(&reverse_inventory),
    )
    .unwrap();
    assert!(facts.mechanics.transitions.is_empty());
    assert!(facts.encoded_exits[0].candidate_transition_ids.is_empty());
}

#[test]
fn imports_l5_human_guard_and_distinguishes_dungeon_from_boss_room_unlock() {
    let content = audited_content();
    let runtime = runtime(&content);
    let dungeon_inventory = l5_boss_door_inventory(false);
    let context = world_context(content.fingerprint.game_data_sha256, &dungeon_inventory);
    let dungeon = ExtractedWorldFacts::build(
        &content,
        &runtime,
        &context,
        std::slice::from_ref(&dungeon_inventory),
    )
    .unwrap();
    let transition = &dungeon.mechanics.transitions[0];
    let PredicateExpression::All { terms } = &transition.activation.hard_guards else {
        panic!("L5 boss door must retain location, boss-key, and form guards")
    };
    assert!(terms.iter().any(|term| {
        matches!(
            term,
            PredicateExpression::Compare {
                left: ValueReference::PlayerForm,
                operator: ComparisonOperator::Equal,
                right: ValueReference::Literal {
                    value: StateValue::Text(form),
                },
            } if form == "human"
        )
    }));
    assert!(matches!(
        transition.activation.effects.as_slice(),
        [
            StateOperation::WriteBoundRaw {
                component_kind: ComponentKind::DungeonMemory,
                binding: ComponentBindingReference::CurrentStage,
                byte_offset: 0x0a,
                mask,
                value,
            },
            StateOperation::SetLocation { location },
        ] if mask == &[0x01]
            && value == &[0x01]
            && location.stage == "D_MN11A"
            && location.room == 50
            && location.spawn == 1
    ));
    assert!(transition.evidence.records.iter().any(|record| {
        record.source_sha256
            == Some(static_digest(
                "9f649b99f027e39f1d39ce066d815a78032b536c4a9a83e0361681af2265102e",
            ))
    }));
    assert_eq!(dungeon.spatial_volumes.len(), 1);
    assert_eq!(dungeon.spatial_planes.len(), 1);
    assert!(matches!(
        &dungeon.spatial_volumes[0].shape,
        SpatialVolumeShape::YawOrientedRectangle {
            origin_xz,
            yaw: 0,
            minimum_local_xz,
            maximum_local_xz,
        } if origin_xz == &[0.0, -5237.0]
            && minimum_local_xz == &[-200.0, -100.0]
            && maximum_local_xz == &[200.0, 100.0]
    ));
    assert_eq!(dungeon.spatial_planes[0].normal, [0.0, 0.0, 1.0]);
    assert_eq!(dungeon.spatial_planes[0].offset, 5237.0);
    assert_eq!(transition.activation.physical_obligation_ids.len(), 4);
    assert!(dungeon.mechanics.obligations.iter().any(|obligation| {
        matches!(
            &obligation.detail,
            ObligationDetail::Interaction {
                required_volumes,
                ..
            } if required_volumes[0].volume_id == "boss-door-check-area"
        )
    }));
    assert!(dungeon.mechanics.obligations.iter().any(|obligation| {
        matches!(
            &obligation.detail,
            ObligationDetail::PlaneSide {
                plane_id,
                relation: crate::state::PlaneRelation::Positive,
            } if plane_id == &dungeon.spatial_planes[0].plane_id
        )
    }));

    let boss_room_inventory = l5_boss_door_inventory(true);
    let context = world_context(content.fingerprint.game_data_sha256, &boss_room_inventory);
    let boss_room = ExtractedWorldFacts::build(
        &content,
        &runtime,
        &context,
        std::slice::from_ref(&boss_room_inventory),
    )
    .unwrap();
    assert!(matches!(
        boss_room.mechanics.transitions[0]
            .activation
            .effects
            .as_slice(),
        [StateOperation::SetLocation { location }]
            if location.stage == "F_SP114"
                && location.room == 1
                && location.layer == 2
                && location.spawn == 11
    ));
}

#[test]
fn imports_keyed_mboss_first_open_and_reopen_as_distinct_branches() {
    let content = audited_content();
    let runtime = runtime(&content);
    let inventory = keyed_mboss_inventory();
    inventory.validate().unwrap();
    let context = world_context(content.fingerprint.game_data_sha256, &inventory);
    let facts = ExtractedWorldFacts::build(
        &content,
        &runtime,
        &context,
        std::slice::from_ref(&inventory),
    )
    .unwrap();

    assert_eq!(facts.schema, EXTRACTED_WORLD_FACTS_SCHEMA);
    assert_eq!(facts.mechanics.transitions.len(), 3);
    assert_eq!(facts.mechanics.obligations.len(), 2);
    assert_eq!(facts.encoded_exits[0].candidate_transition_ids.len(), 3);
    let first_open = facts
        .mechanics
        .transitions
        .iter()
        .find(|transition| {
            transition
                .activation
                .effects
                .iter()
                .any(|effect| matches!(effect, StateOperation::AdjustBoundRawUnsigned { .. }))
        })
        .expect("first keyed opening branch");
    assert_eq!(first_open.transition_kind, TransitionKind::Door);
    assert!(matches!(
        first_open.activation.effects.as_slice(),
        [
            StateOperation::WriteBoundRaw {
                component_kind: ComponentKind::DungeonMemory,
                binding: ComponentBindingReference::CurrentStage,
                byte_offset: 0x08,
                mask,
                value,
            },
            StateOperation::AdjustBoundRawUnsigned {
                component_kind: ComponentKind::DungeonMemory,
                binding: ComponentBindingReference::CurrentStage,
                byte_offset: 0x1c,
                byte_width: 1,
                delta: -1,
            },
            StateOperation::SetLocation { location },
        ] if mask == &[0x20]
            && value == &[0x20]
            && location.stage == "D_MN06B"
            && location.room == 51
    ));
    let PredicateExpression::All { terms } = &first_open.activation.hard_guards else {
        panic!("first opening must retain location, switch, and key guards")
    };
    assert!(terms.iter().any(|term| matches!(
        term,
        PredicateExpression::Compare {
            left: ValueReference::BoundRawBits {
                byte_offset: 0x1c,
                byte_width: 1,
                mask: 0xff,
                ..
            },
            operator: ComparisonOperator::GreaterThan,
            right: ValueReference::Literal {
                value: StateValue::Unsigned(0),
            },
        }
    )));
    assert!(has_small_key_comparison(
        first_open,
        ComparisonOperator::LessThanOrEqual,
        100,
    ));
    let reopen = facts
        .mechanics
        .transitions
        .iter()
        .find(|transition| transition.activation.effects.len() == 1)
        .expect("already-unlocked reopening branch");
    assert!(matches!(
        reopen.activation.effects.as_slice(),
        [StateOperation::SetLocation { location }]
            if location.stage == "D_MN06B" && location.room == 51
    ));
    let high = facts
        .mechanics
        .transitions
        .iter()
        .find(|transition| writes_small_key(transition, 99))
        .expect("high raw key-count clamp branch");
    assert!(has_small_key_comparison(
        high,
        ComparisonOperator::GreaterThan,
        100,
    ));
}

#[test]
fn imports_regular_key_shutter_switch_and_small_key_mutation() {
    let content = audited_content();
    let runtime = runtime(&content);
    let inventory = regular_key_shutter_inventory();
    inventory.validate().unwrap();
    let context = world_context(content.fingerprint.game_data_sha256, &inventory);
    let facts = ExtractedWorldFacts::build(
        &content,
        &runtime,
        &context,
        std::slice::from_ref(&inventory),
    )
    .unwrap();

    assert_eq!(facts.mechanics.transitions.len(), 2);
    assert_eq!(facts.mechanics.obligations.len(), 2);
    let transition = facts
        .mechanics
        .transitions
        .iter()
        .find(|transition| {
            transition
                .activation
                .effects
                .iter()
                .any(|effect| matches!(effect, StateOperation::AdjustBoundRawUnsigned { .. }))
        })
        .expect("ordinary key decrement branch");
    assert_eq!(transition.transition_kind, TransitionKind::ActorDriven);
    assert!(matches!(
        transition.activation.effects.as_slice(),
        [
            StateOperation::WriteBoundRaw {
                byte_offset: 0x0f,
                mask,
                value,
                ..
            },
            StateOperation::AdjustBoundRawUnsigned {
                byte_offset: 0x1c,
                byte_width: 1,
                delta: -1,
                ..
            },
        ] if mask == &[0x08] && value == &[0x08]
    ));
    assert!(has_small_key_comparison(
        transition,
        ComparisonOperator::LessThanOrEqual,
        100,
    ));
    assert!(transition.evidence.records.iter().any(|record| {
        record.source_sha256
            == Some(static_digest(
                "3bff3ce52a0c1660d5ccf0bdcae24b672e50013317b3469698c51e32336c159a",
            ))
    }));
    let high = facts
        .mechanics
        .transitions
        .iter()
        .find(|transition| writes_small_key(transition, 99))
        .expect("high raw key-count clamp branch");
    assert!(has_small_key_comparison(
        high,
        ComparisonOperator::GreaterThan,
        100,
    ));
}

#[test]
fn imports_lakebed_boss_shutter_zero_normal_and_high_small_key_outcomes() {
    let content = audited_content();
    let runtime = runtime(&content);
    let inventory = lakebed_boss_key_shutter_inventory();
    inventory.validate().unwrap();
    let context = world_context(content.fingerprint.game_data_sha256, &inventory);
    let facts = ExtractedWorldFacts::build(
        &content,
        &runtime,
        &context,
        std::slice::from_ref(&inventory),
    )
    .unwrap();

    assert_eq!(facts.mechanics.transitions.len(), 3);
    assert_eq!(facts.mechanics.obligations.len(), 2);
    let positive = facts
        .mechanics
        .transitions
        .iter()
        .find(|transition| {
            transition
                .activation
                .effects
                .iter()
                .any(|effect| matches!(effect, StateOperation::AdjustBoundRawUnsigned { .. }))
        })
        .expect("boss-key opening with a small key");
    assert!(matches!(
        positive.activation.effects.as_slice(),
        [
            StateOperation::WriteBoundRaw {
                byte_offset: 0x11,
                mask,
                value,
                ..
            },
            StateOperation::AdjustBoundRawUnsigned {
                byte_offset: 0x1c,
                byte_width: 1,
                delta: -1,
                ..
            },
        ] if mask == &[0x20] && value == &[0x20]
    ));
    assert!(has_small_key_comparison(
        positive,
        ComparisonOperator::LessThanOrEqual,
        100,
    ));
    let zero = facts
        .mechanics
        .transitions
        .iter()
        .find(|transition| transition.activation.effects.len() == 1)
        .expect("boss-key opening with zero small keys");
    let PredicateExpression::All { terms } = &zero.activation.hard_guards else {
        panic!("zero-key branch must retain boss-key and zero-key guards")
    };
    assert!(terms.iter().any(|term| matches!(
        term,
        PredicateExpression::Compare {
            left: ValueReference::BoundRawBits {
                byte_offset: 0x1d,
                mask: 0x04,
                ..
            },
            right: ValueReference::Literal {
                value: StateValue::Unsigned(0x04),
            },
            ..
        }
    )));
    let high = facts
        .mechanics
        .transitions
        .iter()
        .find(|transition| writes_small_key(transition, 99))
        .expect("high raw key-count clamp branch");
    assert!(has_small_key_comparison(
        high,
        ComparisonOperator::GreaterThan,
        100,
    ));
    assert!(terms.iter().any(|term| matches!(
        term,
        PredicateExpression::Compare {
            left: ValueReference::BoundRawBits {
                byte_offset: 0x1c,
                mask: 0xff,
                ..
            },
            operator: ComparisonOperator::Equal,
            right: ValueReference::Literal {
                value: StateValue::Unsigned(0),
            },
        }
    )));
}

#[test]
fn imports_only_memory_switch_backed_type_zero_koki_gate_unlocks() {
    let content = audited_content();
    let runtime = runtime(&content);
    let inventory = koki_gate_inventory(0x0c);
    inventory.validate().unwrap();
    let context = world_context(content.fingerprint.game_data_sha256, &inventory);
    let facts = ExtractedWorldFacts::build(
        &content,
        &runtime,
        &context,
        std::slice::from_ref(&inventory),
    )
    .unwrap();

    assert_eq!(facts.mechanics.transitions.len(), 2);
    let ordinary = facts
        .mechanics
        .transitions
        .iter()
        .find(|transition| {
            transition
                .activation
                .effects
                .iter()
                .any(|effect| matches!(effect, StateOperation::AdjustBoundRawUnsigned { .. }))
        })
        .expect("ordinary key decrement branch");
    assert!(matches!(
        ordinary.activation.effects.as_slice(),
        [
            StateOperation::AdjustBoundRawUnsigned {
                byte_offset: 0x1c,
                byte_width: 1,
                delta: -1,
                ..
            },
            StateOperation::WriteBoundRaw {
                byte_offset: 0x0a,
                mask,
                value,
                ..
            },
        ] if mask == &[0x10] && value == &[0x10]
    ));
    assert!(has_small_key_comparison(
        ordinary,
        ComparisonOperator::LessThanOrEqual,
        100,
    ));
    let PredicateExpression::All { terms } = &ordinary.activation.hard_guards else {
        panic!("gate must retain location, switch, and key guards")
    };
    assert!(terms.iter().any(|term| matches!(
        term,
        PredicateExpression::All { terms }
            if terms.iter().any(|nested| matches!(
                nested,
                PredicateExpression::Compare {
                    left: ValueReference::LocationLayer,
                    right: ValueReference::Literal {
                        value: StateValue::Signed(0),
                    },
                    ..
                }
            ))
    )));
    let high = facts
        .mechanics
        .transitions
        .iter()
        .find(|transition| writes_small_key(transition, 99))
        .expect("high raw key-count clamp branch");
    assert!(has_small_key_comparison(
        high,
        ComparisonOperator::GreaterThan,
        100,
    ));

    let absent_switch_inventory = koki_gate_inventory(0xff);
    absent_switch_inventory.validate().unwrap();
    let context = world_context(
        content.fingerprint.game_data_sha256,
        &absent_switch_inventory,
    );
    let absent_switch = ExtractedWorldFacts::build(
        &content,
        &runtime,
        &context,
        std::slice::from_ref(&absent_switch_inventory),
    )
    .unwrap();
    assert!(absent_switch.mechanics.transitions.is_empty());
    assert!(absent_switch.mechanics.obligations.is_empty());
}

#[test]
fn imports_fsp109_rider_gate_unlock_and_m035_bypass_without_conflating_them() {
    let content = audited_content();
    let runtime = runtime(&content);
    let inventory = rider_gate_inventory();
    inventory.validate().unwrap();
    let context = world_context(content.fingerprint.game_data_sha256, &inventory);
    let facts = ExtractedWorldFacts::build(
        &content,
        &runtime,
        &context,
        std::slice::from_ref(&inventory),
    )
    .unwrap();

    assert_eq!(facts.mechanics.transitions.len(), 4);
    assert_eq!(facts.mechanics.obligations.len(), 3);
    let ordinary =
        facts
            .mechanics
            .transitions
            .iter()
            .find(|transition| {
                transition.label.contains("keyed rider-gate unlock")
                    && transition.activation.effects.iter().any(|effect| {
                        matches!(effect, StateOperation::AdjustBoundRawUnsigned { .. })
                    })
            })
            .unwrap();
    assert!(predicate_has_persistent_event_bit(
        &ordinary.activation.hard_guards,
        false,
    ));
    assert!(has_small_key_comparison(
        ordinary,
        ComparisonOperator::GreaterThan,
        0,
    ));
    assert!(matches!(
        ordinary.activation.effects.as_slice(),
        [
            StateOperation::AdjustBoundRawUnsigned {
                byte_offset: 0x1c,
                delta: -1,
                ..
            },
            StateOperation::WriteBoundRaw {
                byte_offset: 0x16,
                mask,
                value,
                ..
            },
        ] if mask == &[0x08] && value == &[0x08]
    ));

    let event_bypass = facts
        .mechanics
        .transitions
        .iter()
        .find(|transition| transition.label.contains("M_035 forced-open"))
        .unwrap();
    assert!(event_bypass.activation.effects.is_empty());
    assert!(predicate_has_persistent_event_bit(
        &event_bypass.activation.hard_guards,
        true,
    ));
    assert!(!has_small_key_comparison(
        event_bypass,
        ComparisonOperator::GreaterThan,
        0,
    ));

    let unlocked = facts
        .mechanics
        .transitions
        .iter()
        .find(|transition| transition.label.contains("set-switch physical passage"))
        .unwrap();
    assert!(unlocked.activation.effects.is_empty());
    assert!(predicate_has_persistent_event_bit(
        &unlocked.activation.hard_guards,
        false,
    ));

    let mut fsp121 = inventory;
    fsp121.stage = "F_SP121".into();
    fsp121.sources[1].scope.room = Some(3);
    fsp121.placements[0].scope.room = Some(3);
    fsp121.placements[0].parameters = (fsp121.placements[0].parameters & !0xff) | 0x82;
    let context = world_context(content.fingerprint.game_data_sha256, &fsp121);
    let imported =
        ExtractedWorldFacts::build(&content, &runtime, &context, std::slice::from_ref(&fsp121))
            .unwrap();
    assert_eq!(imported.mechanics.transitions.len(), 4);
    assert_eq!(imported.mechanics.obligations.len(), 3);
    let unlock = imported
        .mechanics
        .transitions
        .iter()
        .find(|transition| transition.label.contains("keyed rider-gate unlock"))
        .unwrap();
    assert!(unlock.activation.effects.iter().any(|effect| matches!(
        effect,
        StateOperation::WriteBoundRaw {
            component_kind: ComponentKind::Custom { id },
            binding: ComponentBindingReference::CurrentStage,
            byte_offset: 2,
            mask,
            value,
        } if id == DUNGEON_SESSION_SWITCH_LABEL_KIND && mask == &[1] && value == &[1]
    )));

    let mut wrong_room = fsp121;
    wrong_room.sources[1].scope.room = Some(4);
    wrong_room.placements[0].scope.room = Some(4);
    let context = world_context(content.fingerprint.game_data_sha256, &wrong_room);
    let excluded = ExtractedWorldFacts::build(
        &content,
        &runtime,
        &context,
        std::slice::from_ref(&wrong_room),
    )
    .unwrap();
    assert!(excluded.mechanics.transitions.is_empty());
}

#[test]
fn imports_rsp116_wolf_chain_writer_and_vshuter_consumer_as_causal_pair() {
    let content = audited_content();
    let runtime = runtime(&content);
    let mut inventory = regular_key_shutter_inventory();
    inventory.stage = "R_SP116".into();
    inventory.sources[1].scope.room = Some(6);
    let placement = &mut inventory.placements[0];
    placement.name = "vshuter".into();
    placement.scope.room = Some(6);
    placement.parameters = 0x00ff_03ef;
    let mut chain = placement.clone();
    chain.stable_id = format!("{}/wchain", chain.stable_id);
    chain.name = "Wchain".into();
    chain.parameters = 0x0000_0fef;
    chain.raw_hex = "57636861696e000000000fefbefd6d7fc4fa0000c4a71af4000000000000ffff".into();
    inventory.placements.push(chain);
    let context = world_context(content.fingerprint.game_data_sha256, &inventory);
    let facts = ExtractedWorldFacts::build(
        &content,
        &runtime,
        &context,
        std::slice::from_ref(&inventory),
    )
    .unwrap();

    assert_eq!(facts.mechanics.transitions.len(), 2);
    assert_eq!(facts.mechanics.obligations.len(), 4);
    let writer = facts
        .mechanics
        .transitions
        .iter()
        .find(|transition| transition.label.contains("wolf-chain pull"))
        .unwrap();
    assert!(matches!(
        writer.activation.effects.as_slice(),
        [StateOperation::WriteBoundRaw {
            component_kind: ComponentKind::Custom { id },
            binding: ComponentBindingReference::CurrentRoom,
            byte_offset: 0xef,
            mask,
            value,
        }] if id == ROOM_SWITCH_LABEL_KIND && mask == &[1] && value == &[1]
    ));
    assert!(matches!(
        &writer.activation.hard_guards,
        PredicateExpression::All { terms }
            if terms.iter().any(|term| matches!(
                term,
                PredicateExpression::Compare {
                    left: ValueReference::PlayerForm,
                    operator: ComparisonOperator::Equal,
                    right: ValueReference::Literal {
                        value: StateValue::Text(form),
                    },
                } if form == "wolf"
            ))
    ));

    let passage = facts
        .mechanics
        .transitions
        .iter()
        .find(|transition| transition.label.contains("externally switched passage"))
        .unwrap();
    assert!(passage.activation.effects.is_empty());
    assert!(matches!(
        &passage.activation.hard_guards,
        PredicateExpression::All { terms }
            if terms.iter().any(|term| matches!(
                term,
                PredicateExpression::Compare {
                    left: ValueReference::BoundRawBits {
                        component_kind: ComponentKind::Custom { id },
                        binding: ComponentBindingReference::CurrentRoom,
                        byte_offset: 0xef,
                        byte_width: 1,
                        mask: 1,
                    },
                    operator: ComparisonOperator::Equal,
                    right: ValueReference::Literal {
                        value: StateValue::Unsigned(1),
                    },
                } if id == ROOM_SWITCH_LABEL_KIND
            ))
    ));
}

#[test]
fn imports_fsp118_caravan_key_opening_and_boar_bypass_as_transient_branches() {
    let content = audited_content();
    let runtime = runtime(&content);
    let inventory = caravan_gate_inventory();
    inventory.validate().unwrap();
    let context = world_context(content.fingerprint.game_data_sha256, &inventory);
    let facts = ExtractedWorldFacts::build(
        &content,
        &runtime,
        &context,
        std::slice::from_ref(&inventory),
    )
    .unwrap();

    assert_eq!(facts.mechanics.transitions.len(), 3);
    assert_eq!(facts.mechanics.obligations.len(), 3);
    let ordinary = facts
        .mechanics
        .transitions
        .iter()
        .find(|transition| transition.label.ends_with("transient key opening"))
        .unwrap();
    assert!(has_small_key_comparison(
        ordinary,
        ComparisonOperator::GreaterThan,
        0,
    ));
    assert!(matches!(
        ordinary.activation.effects.as_slice(),
        [StateOperation::AdjustBoundRawUnsigned {
            byte_offset: 0x1c,
            delta: -1,
            ..
        }]
    ));
    assert!(
        !ordinary
            .activation
            .effects
            .iter()
            .any(|effect| matches!(effect, StateOperation::WriteBoundRaw { .. }))
    );

    let high = facts
        .mechanics
        .transitions
        .iter()
        .find(|transition| transition.label.contains("clamped to 99"))
        .unwrap();
    assert!(writes_small_key(high, 99));
    let boar = facts
        .mechanics
        .transitions
        .iter()
        .find(|transition| transition.label.contains("boar destruction bypass"))
        .unwrap();
    assert!(boar.activation.effects.is_empty());
    assert!(!has_small_key_comparison(
        boar,
        ComparisonOperator::GreaterThan,
        0,
    ));
    assert_eq!(boar.activation.physical_obligation_ids.len(), 1);

    let mut unrelated = inventory;
    unrelated.stage = "F_SP117".into();
    let context = world_context(content.fingerprint.game_data_sha256, &unrelated);
    let excluded = ExtractedWorldFacts::build(
        &content,
        &runtime,
        &context,
        std::slice::from_ref(&unrelated),
    )
    .unwrap();
    assert!(excluded.mechanics.transitions.is_empty());
}

#[test]
fn imports_l7_bridge_demo_as_two_distinct_scls_backed_actor_transitions() {
    let content = audited_content();
    let runtime = runtime(&content);
    let inventory = l7_bridge_demo_inventory();
    inventory.validate().unwrap();
    let context = world_context(content.fingerprint.game_data_sha256, &inventory);
    let facts = ExtractedWorldFacts::build(
        &content,
        &runtime,
        &context,
        std::slice::from_ref(&inventory),
    )
    .unwrap();

    assert_eq!(facts.mechanics.transitions.len(), 2);
    assert_eq!(facts.mechanics.obligations.len(), 3);
    let enter = facts
        .mechanics
        .transitions
        .iter()
        .find(|transition| transition.label.contains("SCLS 6"))
        .unwrap();
    assert_eq!(enter.transition_kind, TransitionKind::ActorDriven);
    assert!(has_small_key_comparison(
        enter,
        ComparisonOperator::GreaterThan,
        0,
    ));
    assert!(!enter.activation.effects.iter().any(|effect| matches!(
        effect,
        StateOperation::AdjustBoundRawUnsigned { .. }
            | StateOperation::WriteBoundRaw {
                byte_offset: 0x1c,
                ..
            }
    )));
    assert!(enter.activation.effects.iter().any(|effect| matches!(
        effect,
        StateOperation::SetLocation { location }
            if location.stage == "D_MN07"
                && location.room == 6
                && location.layer == 3
                && location.spawn == 7
    )));
    assert!(predicate_has_layer_comparison(
        &enter.activation.hard_guards,
        ComparisonOperator::NotEqual,
        3,
    ));

    let destroy = facts
        .mechanics
        .transitions
        .iter()
        .find(|transition| transition.label.contains("SCLS 7"))
        .unwrap();
    assert!(!has_small_key_comparison(
        destroy,
        ComparisonOperator::GreaterThan,
        0,
    ));
    assert!(predicate_has_layer_comparison(
        &destroy.activation.hard_guards,
        ComparisonOperator::Equal,
        3,
    ));
    assert!(matches!(
        destroy.activation.effects.as_slice(),
        [
            StateOperation::WriteBoundRaw {
                byte_offset: 0x08,
                mask,
                value,
                ..
            },
            StateOperation::SetLocation { location },
        ] if mask == &[0x01]
            && value == &[0x01]
            && location.stage == "D_MN07"
            && location.room == 6
            && location.layer == -1
            && location.spawn == 8
    ));
    for record_index in [6, 7] {
        let exit = facts
            .encoded_exits
            .iter()
            .find(|exit| {
                exit.source_record_id
                    .ends_with(&format!("record/{record_index}"))
            })
            .unwrap();
        assert_eq!(exit.candidate_transition_ids.len(), 1);
    }
}

#[test]
fn imports_wrapped_type_zero_shutter_and_each_mboss_event_resource() {
    let scope = ContextScope {
        selectors: vec![ContextSelector::Exact {
            context: ExactContext {
                content_sha256: Digest([0x11; 32]),
                runtime_configuration_sha256: Digest([0x22; 32]),
            },
        }],
    };
    let inventory = regular_key_shutter_inventory();
    let mut shutter = inventory.placements[0].clone();
    shutter.parameters = 0x80ff_ff2b;
    let imported =
        import_gz2e01_keyed_actor_actions(&inventory, &shutter, &scope, Digest([0x33; 32]))
            .unwrap()
            .expect("authored 0xff wraps to the supported runtime type zero");
    assert_eq!(imported.transitions.len(), 2);
    assert!(
        imported.transitions[0]
            .evidence
            .records
            .iter()
            .any(|record| {
                record.source_sha256
                    == Some(static_digest(
                        "8676effbd561ba65f8e4a8b9493aa6b60072d40f72a8e240b2ffa9c5550b40fa",
                    ))
            })
    );

    let inventory = keyed_mboss_inventory();
    for (name, event_sha256) in [
        (
            "L7door",
            "7de6bfac10e3ca6c3f6bc88a83815972d3397fd3488b067398cdd8cb0ea0cce4",
        ),
        (
            "L8Mdoor",
            "b079b8b284208582d9a37b50bd94f13400530abca75db0771147a646a8d83627",
        ),
    ] {
        let mut placement = inventory.placements[0].clone();
        placement.name = name.into();
        let imported =
            import_gz2e01_keyed_actor_actions(&inventory, &placement, &scope, Digest([0x33; 32]))
                .unwrap()
                .expect("audited keyed mini-boss alias");
        assert_eq!(imported.transitions.len(), 3);
        assert!(
            imported.transitions[0]
                .evidence
                .records
                .iter()
                .any(|record| record.source_sha256 == Some(static_digest(event_sha256)))
        );
    }
}

#[test]
fn excludes_unaudited_keyed_family_bypasses_and_non_memory_switches() {
    let scope = ContextScope {
        selectors: vec![ContextSelector::Exact {
            context: ExactContext {
                content_sha256: Digest([0x11; 32]),
                runtime_configuration_sha256: Digest([0x22; 32]),
            },
        }],
    };
    let inventory = regular_key_shutter_inventory();
    let mut placement = inventory.placements[0].clone();
    for name in ["vshuter"] {
        placement.name = name.into();
        assert!(
            import_gz2e01_keyed_actor_actions(&inventory, &placement, &scope, Digest([0x33; 32]),)
                .unwrap()
                .is_none()
        );
    }

    placement = inventory.placements[0].clone();
    placement.parameters &= 0x7fff_ffff;
    assert!(
        import_gz2e01_keyed_actor_actions(&inventory, &placement, &scope, Digest([0x33; 32]),)
            .unwrap()
            .is_none()
    );

    let gate_inventory = koki_gate_inventory(0x82);
    let mut gate = gate_inventory.placements[0].clone();
    assert!(
        import_gz2e01_keyed_actor_actions(&gate_inventory, &gate, &scope, Digest([0x33; 32]),)
            .unwrap()
            .is_none()
    );
    gate.parameters = (gate.parameters & !(0x0f << 16)) | (1 << 16);
    assert!(
        import_gz2e01_keyed_actor_actions(&gate_inventory, &gate, &scope, Digest([0x33; 32]),)
            .unwrap()
            .is_none()
    );
}

#[test]
fn encoded_exits_reference_every_location_changing_world_transition() {
    let content = audited_content();
    let runtime = runtime(&content);

    let gate_inventory = koki_gate_inventory(0x0c);
    let context = world_context(content.fingerprint.game_data_sha256, &gate_inventory);
    let mut actor_facts = ExtractedWorldFacts::build(
        &content,
        &runtime,
        &context,
        std::slice::from_ref(&gate_inventory),
    )
    .unwrap();
    actor_facts.mechanics.transitions[0].transition_kind = TransitionKind::Door;
    assert_eq!(
        actor_facts.validate().unwrap_err().field(),
        "mechanics.transitions"
    );

    let door_inventory = keyed_mboss_inventory();
    let context = world_context(content.fingerprint.game_data_sha256, &door_inventory);
    let mut door_facts = ExtractedWorldFacts::build(
        &content,
        &runtime,
        &context,
        std::slice::from_ref(&door_inventory),
    )
    .unwrap();
    door_facts.mechanics.transitions[0].transition_kind = TransitionKind::ActorDriven;
    door_facts.validate().unwrap();
    let transition_id = door_facts.mechanics.transitions[0].id.clone();
    door_facts
        .encoded_exits
        .iter_mut()
        .find(|exit| exit.candidate_transition_ids.contains(&transition_id))
        .unwrap()
        .candidate_transition_ids
        .retain(|id| id != &transition_id);
    assert_eq!(
        door_facts.validate().unwrap_err().field(),
        "mechanics.transitions"
    );
}

#[test]
fn keeps_unjoined_scls_as_encoded_fact_without_inventing_transition() {
    let content = content();
    let runtime = runtime(&content);
    let inventory = inventory(false);
    inventory.validate().unwrap();
    let context = world_context(Digest([2; 32]), &inventory);
    let facts = ExtractedWorldFacts::build(
        &content,
        &runtime,
        &context,
        std::slice::from_ref(&inventory),
    )
    .unwrap();
    assert!(facts.mechanics.transitions.is_empty());
    assert!(facts.mechanics.obligations.is_empty());
    assert!(facts.encoded_exits[0].candidate_transition_ids.is_empty());
}

#[test]
fn exact_content_and_world_context_must_agree() {
    let content = content();
    let runtime = runtime(&content);
    let inventory = inventory(false);
    let context = world_context(Digest([9; 32]), &inventory);
    assert_eq!(
        ExtractedWorldFacts::build(
            &content,
            &runtime,
            &context,
            std::slice::from_ref(&inventory),
        )
        .unwrap_err()
        .field(),
        "world_context.game_data_sha256"
    );
}
