
use super::*;
use crate::world_geometry::{CollisionCode, KclAuthoredPrism, KclSourceIndices};

fn canonical_inventory() -> WorldInventory {
    let stage_archive = sha256(b"stage-archive");
    let stage_data = sha256(b"stage-data");
    let room_archive = sha256(b"room-archive");
    let room_data = sha256(b"room-data");
    let kcl = sha256(b"room-kcl");
    let plc = sha256(b"room-plc");
    let code = CollisionCode::decode([0x3f, 0, 0, 0, 0]);
    WorldInventory {
        schema: WORLD_INVENTORY_SCHEMA.into(),
        stage: "F_TEST".into(),
        sources: vec![
            WorldSource {
                scope: SourceScope {
                    kind: SourceKind::Stage,
                    room: None,
                },
                archive_sha256: stage_archive,
                stage_data_path: "stage.dzs".into(),
                stage_data_sha256: stage_data,
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
                archive_sha256: room_archive,
                stage_data_path: "room.dzr".into(),
                stage_data_sha256: room_data,
                kcl_path: Some("room.kcl".into()),
                kcl_sha256: Some(kcl),
                plc_path: Some("room.plc".into()),
                plc_sha256: Some(plc),
                addressable_prisms: 1,
            },
        ],
        chunks: Vec::new(),
        placements: Vec::new(),
        player_spawns: Vec::new(),
        exits: Vec::new(),
        paths: Vec::new(),
        path_points: Vec::new(),
        collisions: vec![CollisionInventoryRecord {
            room: 0,
            prism: KclInventoryPrism {
                authored: KclAuthoredPrism {
                    stable_id: format!("kcl-sha256:{kcl}/plc-sha256:{plc}/prism/1"),
                    prism_index: 1,
                    height: 0.0,
                    source_indices: KclSourceIndices {
                        position: 0,
                        face_normal: 0,
                        edge_normal_1: 0,
                        edge_normal_2: 0,
                        edge_normal_3: 0,
                    },
                    attribute: 0,
                    code,
                },
                reconstruction: KclReconstruction::Degenerate {
                    reason: "fixture".into(),
                },
            },
        }],
        load_triggers: Vec::new(),
    }
}

fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}

fn put_f32(bytes: &mut [u8], offset: usize, value: f32) {
    put_u32(bytes, offset, value.to_bits());
}

fn stage_data() -> Vec<u8> {
    let directory = 4 + 3 * STAGE_CHUNK_HEADER_SIZE;
    let actor_offset = directory;
    let exit_offset = actor_offset + PLACEMENT_SIZE;
    let player_offset = exit_offset + SCLS_SIZE;
    let mut bytes = vec![0; player_offset + PLACEMENT_SIZE];
    put_u32(&mut bytes, 0, 3);
    for (index, (tag, count, offset)) in [
        (b"ACT3", 1_u32, actor_offset),
        (b"SCLS", 1, exit_offset),
        (b"PLYR", 1, player_offset),
    ]
    .into_iter()
    .enumerate()
    {
        let node = 4 + index * STAGE_CHUNK_HEADER_SIZE;
        bytes[node..node + 4].copy_from_slice(tag);
        put_u32(&mut bytes, node + 4, count);
        put_u32(&mut bytes, node + 8, offset as u32);
    }
    bytes[actor_offset..actor_offset + 8].copy_from_slice(b"CamArea\0");
    put_u32(&mut bytes, actor_offset + 8, 0x1234_5678);
    put_f32(&mut bytes, actor_offset + 12, 1.0);
    put_f32(&mut bytes, actor_offset + 16, 2.0);
    put_f32(&mut bytes, actor_offset + 20, 3.0);
    put_u16(&mut bytes, actor_offset + 24, 4);
    put_u16(&mut bytes, actor_offset + 26, 5);
    put_u16(&mut bytes, actor_offset + 28, 6);
    put_u16(&mut bytes, actor_offset + 30, 7);
    bytes[exit_offset..exit_offset + 8].copy_from_slice(b"F_SP104\0");
    bytes[exit_offset + 8] = 0;
    bytes[exit_offset + 9] = 1;
    bytes[exit_offset + 10] = 0xf0;
    bytes[exit_offset + 11] = 0x9f;
    bytes[exit_offset + 12] = 19;
    bytes[player_offset..player_offset + 8].copy_from_slice(b"Link\0\0\0\0");
    bytes
}

fn path_stage_data() -> Vec<u8> {
    let directory = 4 + 2 * STAGE_CHUNK_HEADER_SIZE;
    let path_offset = directory;
    let point_offset = path_offset + RPAT_SIZE;
    let mut bytes = vec![0; point_offset + 2 * RPPN_SIZE];
    put_u32(&mut bytes, 0, 2);
    for (index, (tag, count, offset)) in [(b"RPAT", 1_u32, path_offset), (b"RPPN", 2, point_offset)]
        .into_iter()
        .enumerate()
    {
        let node = 4 + index * STAGE_CHUNK_HEADER_SIZE;
        bytes[node..node + 4].copy_from_slice(tag);
        put_u32(&mut bytes, node + 4, count);
        put_u32(&mut bytes, node + 8, offset as u32);
    }
    put_u16(&mut bytes, path_offset, 2);
    put_u16(&mut bytes, path_offset + 2, u16::MAX);
    bytes[path_offset + 4] = 7;
    bytes[path_offset + 5] = 1;
    bytes[path_offset + 6] = u8::MAX;
    bytes[path_offset + 7] = 9;
    put_u32(&mut bytes, path_offset + 8, 0);
    for (index, position) in [[1.0_f32, 2.0, 3.0], [4.0, 5.0, 6.0]]
        .into_iter()
        .enumerate()
    {
        let offset = point_offset + index * RPPN_SIZE;
        bytes[offset..offset + 4].copy_from_slice(&[1, 2, 3, 4]);
        put_f32(&mut bytes, offset + 4, position[0]);
        put_f32(&mut bytes, offset + 8, position[1]);
        put_f32(&mut bytes, offset + 12, position[2]);
    }
    bytes
}

#[test]
fn decodes_authored_records_with_structural_ids_and_raw_bytes() {
    let bytes = stage_data();
    let digest = sha256(&bytes);
    let scope = SourceScope {
        kind: SourceKind::Room,
        room: Some(1),
    };
    let decoded = decode_stage_data(&bytes, digest, scope, "dzr").unwrap();
    assert_eq!(decoded.placements.len(), 1);
    let actor = &decoded.placements[0];
    assert_eq!(actor.name, "CamArea");
    assert_eq!(actor.layer, Some(3));
    assert_eq!(actor.parameters, 0x1234_5678);
    assert_eq!(
        actor.position,
        Vec3 {
            x: 1.0,
            y: 2.0,
            z: 3.0
        }
    );
    assert_eq!(actor.angle, [4, 5, 6]);
    assert_eq!(actor.set_id, 7);
    assert_eq!(actor.raw_hex.len(), PLACEMENT_SIZE * 2);
    assert!(actor.stable_id.starts_with("dzr-sha256:"));
    assert_eq!(decoded.player_spawns.len(), 1);
    assert_eq!(decoded.exits.len(), 1);
    let exit = &decoded.exits[0];
    assert_eq!(exit.destination_stage, "F_SP104");
    assert_eq!(exit.destination_room, 1);
    assert_eq!(exit.destination_layer, -1);
    assert_eq!(exit.wipe, 19);
    assert_eq!(exit.wipe_time, 4);
    assert_eq!(exit.time_hour, -1);
}

#[test]
fn decodes_authored_paths_with_checked_point_spans() {
    let bytes = path_stage_data();
    let digest = sha256(&bytes);
    let scope = SourceScope {
        kind: SourceKind::Room,
        room: Some(1),
    };
    let decoded = decode_stage_data(&bytes, digest, scope, "dzr").unwrap();
    assert_eq!(decoded.paths.len(), 1);
    assert_eq!(decoded.path_points.len(), 2);
    let path = &decoded.paths[0];
    assert_eq!(path.point_count, 2);
    assert_eq!(path.first_point_index, 0);
    assert_eq!(path.next_path_index, None);
    assert_eq!(path.path_argument, 7);
    assert!(path.closed);
    assert_eq!(path.switch_no, None);
    assert_eq!(path.unknown_07, 9);
    assert_eq!(decoded.path_points[0].arguments, [4, 1, 2, 3]);
    assert_eq!(
        decoded.path_points[1].position,
        Vec3 {
            x: 4.0,
            y: 5.0,
            z: 6.0
        }
    );
    assert_eq!(decoded.chunks[0].recognized_record_size, Some(RPAT_SIZE));
    assert_eq!(decoded.chunks[1].recognized_record_size, Some(RPPN_SIZE));

    let source_by_data = BTreeMap::from([(digest, scope)]);
    validate_authored_paths(
        &decoded.paths,
        &decoded.path_points,
        &source_by_data,
        &decoded.chunks,
    )
    .unwrap();

    let mut invalid = decoded.paths.clone();
    invalid[0].point_count = 3;
    assert!(
        validate_authored_paths(
            &invalid,
            &decoded.path_points,
            &source_by_data,
            &decoded.chunks,
        )
        .is_err()
    );
}

#[test]
fn rejects_duplicate_chunks_bad_ranges_and_nonfinite_positions() {
    let scope = SourceScope {
        kind: SourceKind::Stage,
        room: None,
    };
    let mut duplicate = stage_data();
    duplicate[4 + STAGE_CHUNK_HEADER_SIZE..4 + STAGE_CHUNK_HEADER_SIZE + 4]
        .copy_from_slice(b"ACT3");
    assert!(decode_stage_data(&duplicate, sha256(&duplicate), scope, "dzs").is_err());

    let mut bad_range = stage_data();
    put_u32(&mut bad_range, 4 + 8, u32::MAX);
    assert!(decode_stage_data(&bad_range, sha256(&bad_range), scope, "dzs").is_err());

    let mut nonfinite = stage_data();
    let actor_offset = 4 + 3 * STAGE_CHUNK_HEADER_SIZE;
    put_u32(&mut nonfinite, actor_offset + 12, f32::NAN.to_bits());
    assert!(decode_stage_data(&nonfinite, sha256(&nonfinite), scope, "dzs").is_err());
}

#[test]
fn stable_ids_change_with_source_content() {
    let first = stage_data();
    let mut second = first.clone();
    second[4 + 3 * STAGE_CHUNK_HEADER_SIZE + 8] ^= 1;
    let scope = SourceScope {
        kind: SourceKind::Room,
        room: Some(1),
    };
    let first_id = decode_stage_data(&first, sha256(&first), scope, "dzr")
        .unwrap()
        .placements[0]
        .stable_id
        .clone();
    let second_id = decode_stage_data(&second, sha256(&second), scope, "dzr")
        .unwrap()
        .placements[0]
        .stable_id
        .clone();
    assert_ne!(first_id, second_id);
}

#[test]
fn canonical_inventory_round_trips_and_rejects_alternate_or_tampered_bytes() {
    let inventory = canonical_inventory();
    inventory.validate().unwrap();
    let bytes = inventory.canonical_bytes().unwrap();
    assert_eq!(WorldInventory::decode_canonical(&bytes).unwrap(), inventory);

    let mut legacy = inventory.clone();
    legacy.schema = WORLD_INVENTORY_SCHEMA_V1.into();
    let legacy_bytes = legacy.canonical_bytes().unwrap();
    assert!(!legacy_bytes.windows(7).any(|bytes| bytes == b"\"paths\""));
    assert_eq!(
        WorldInventory::decode_canonical(&legacy_bytes).unwrap(),
        legacy
    );

    let pretty = serde_json::to_vec_pretty(&inventory).unwrap();
    assert!(WorldInventory::decode_canonical(&pretty).is_err());

    let mut tampered = inventory.clone();
    tampered.collisions[0].prism.authored.code.exit_id = 0;
    let tampered_bytes = serde_json::to_vec(&tampered).unwrap();
    assert!(WorldInventory::decode_canonical(&tampered_bytes).is_err());

    let mut reordered = inventory;
    reordered.sources.swap(0, 1);
    assert!(reordered.validate().is_err());
}

#[test]
fn real_f_sp103_inventory_matches_content_golden_when_disc_is_present() {
    let stage_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../..")
        .join("orig/GZ2E01/files/res/Stage/F_SP103");
    if !stage_dir.is_dir() {
        eprintln!("skipping F_SP103 content golden: original disc data is absent");
        return;
    }

    let inventory = WorldInventory::build(&stage_dir, "F_SP103").unwrap();
    assert_eq!(inventory.sources.len(), 3);
    assert_eq!(inventory.placements.len(), 1_442);
    assert_eq!(inventory.player_spawns.len(), 48);
    assert_eq!(inventory.exits.len(), 44);
    assert_eq!(inventory.paths.len(), 82);
    assert_eq!(inventory.path_points.len(), 426);
    assert_eq!(inventory.collisions.len(), 10_794);
    assert_eq!(inventory.load_triggers.len(), 40);
    assert_eq!(
        inventory.digest().unwrap().to_string(),
        "e4f63e67da30035c8d15cc92595bc384dcc84adbe7a5d361acbcb0302bdcd743"
    );

    let degenerate = inventory
        .collisions
        .iter()
        .filter(|collision| {
            matches!(
                collision.prism.reconstruction,
                KclReconstruction::Degenerate { .. }
            )
        })
        .count();
    assert_eq!(degenerate, 4);

    let route_trigger = inventory
        .load_triggers
        .iter()
        .find(|trigger| trigger.room == 1 && trigger.collision_id.ends_with("/prism/2217"))
        .expect("room 1 prism 2217 must resolve to an authored load trigger");
    assert_eq!(route_trigger.collision_exit_id, 1);
    assert_eq!(route_trigger.destination_stage, "F_SP104");
    assert_eq!(route_trigger.destination_room, 1);
    assert_eq!(route_trigger.destination_layer, -1);
    assert_eq!(route_trigger.destination_point, 0);

    assert!(inventory.exits.iter().any(|exit| {
        exit.scope.room == Some(1)
            && exit.record_index == 1
            && exit.destination_stage == "F_SP104"
            && exit.wipe == 19
            && exit.wipe_time == 4
            && exit.time_hour == -1
    }));
}

#[test]
fn real_f_sp104_main_path_is_available_when_disc_is_present() {
    let stage_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../..")
        .join("orig/GZ2E01/files/res/Stage/F_SP104");
    if !stage_dir.is_dir() {
        eprintln!("skipping F_SP104 path golden: original disc data is absent");
        return;
    }

    let inventory = WorldInventory::build(&stage_dir, "F_SP104").unwrap();
    assert_eq!(inventory.paths.len(), 18);
    assert_eq!(inventory.path_points.len(), 81);
    assert_eq!(
        inventory.digest().unwrap().to_string(),
        "833ed87c4e4a82b2a110ba637ce71dd5f159ddddbe7e04e4e79f491586dca6fb"
    );
    let route = inventory
        .paths
        .iter()
        .find(|path| path.scope.room == Some(1) && path.record_index == 14)
        .expect("F_SP104 room path 14 must be decoded");
    assert_eq!(route.point_count, 8);
    let source_points = inventory
        .path_points
        .iter()
        .filter(|point| point.source_sha256 == route.source_sha256)
        .collect::<Vec<_>>();
    let points = &source_points
        [route.first_point_index..route.first_point_index + route.point_count as usize];
    assert_eq!(
        points[0].position,
        Vec3 {
            x: 300.0,
            y: 270.81253,
            z: -3950.0
        }
    );
    assert_eq!(
        points[7].position,
        Vec3 {
            x: -441.90887,
            y: 314.0304,
            z: -19270.963
        }
    );
}
