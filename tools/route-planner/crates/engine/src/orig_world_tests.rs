use super::*;
use crate::identity::{
    CONTENT_IDENTITY_SCHEMA, ContentFingerprint, ContentIdentity, GamePlatform, GameRegion,
    RUNTIME_CONFIGURATION_SCHEMA, RuntimeConfiguration,
};
use crate::orig_extraction::{
    ExtractedCamera, ExtractedCameraArrow, ExtractedLoadedRoom, ExtractedPath, ExtractedPathPoint,
    ExtractedRoomRead, ExtractedStageChunk, ExtractedStageData, extract_unique_rarc_resource,
    parse_stage_data,
};
use crate::world_import::ExtractedWorldFacts;
use std::fs;
use std::path::{Path, PathBuf};

fn digest(byte: u8) -> Digest {
    Digest([byte; 32])
}

fn placement_raw(name: &str, parameters: u32, position: [f32; 3]) -> String {
    let mut bytes = [0_u8; 32];
    bytes[..name.len()].copy_from_slice(name.as_bytes());
    bytes[8..12].copy_from_slice(&parameters.to_be_bytes());
    for (index, value) in position.into_iter().enumerate() {
        bytes[12 + index * 4..16 + index * 4].copy_from_slice(&value.to_bits().to_be_bytes());
    }
    bytes[24..26].copy_from_slice(&1_i16.to_be_bytes());
    bytes[26..28].copy_from_slice(&2_i16.to_be_bytes());
    bytes[28..30].copy_from_slice(&3_i16.to_be_bytes());
    bytes[30..32].copy_from_slice(&4_u16.to_be_bytes());
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn placement(tag: &str, index: u32, name: &str, parameters: u32) -> ExtractedActorPlacement {
    ExtractedActorPlacement {
        chunk_tag: tag.into(),
        record_index: index,
        layer: layer_for_tag(tag),
        name: name.into(),
        parameters,
        position: [1.0, 2.0, 3.0],
        angle: [1, 2, 3],
        set_id: 4,
        scale_raw: None,
        raw_hex: placement_raw(name, parameters, [1.0, 2.0, 3.0]),
    }
}

fn archive(
    relative_path: &str,
    resource_name: &str,
    resource_sha256: Digest,
    stage: ExtractedStageData,
) -> ExtractedOrigStageArchive {
    ExtractedOrigStageArchive {
        relative_path: relative_path.into(),
        archive_sha256: digest(resource_sha256.0[0].wrapping_add(1)),
        resource_name: resource_name.into(),
        resource_sha256,
        stage,
    }
}

#[test]
fn converts_native_stage_records_without_collision_claims() {
    let mut scls = [0_u8; 13];
    scls[..6].copy_from_slice(b"F_SP00");
    scls[8] = 7;
    scls[9] = 2;
    scls[10] = 0xf0;
    scls[11] = 0xff;
    scls[12] = 15;
    let scls_hex = scls.iter().map(|byte| format!("{byte:02x}")).collect();
    let stage = archive(
        "files/res/Stage/F_SP00/STG_00.arc",
        "stage.dzs",
        digest(2),
        ExtractedStageData {
            chunks: vec![
                ExtractedStageChunk {
                    tag: "ACTR".into(),
                    record_count: 1,
                    data_offset: 64,
                    recognized_record_size: Some(32),
                },
                ExtractedStageChunk {
                    tag: "RTBL".into(),
                    record_count: 1,
                    data_offset: 96,
                    recognized_record_size: None,
                },
            ],
            stage_information: None,
            room_transforms: Vec::new(),
            file_lists: Vec::new(),
            room_read_table: vec![ExtractedRoomRead {
                room_index: 0,
                record_offset: 100,
                room_list_offset: 128,
                reverb: 5,
                reverb_raw: 5,
                time_pass: 3,
                vrbox_enabled: true,
                flags_raw: 0x0b,
                padding: 0,
                load_rooms: vec![ExtractedLoadedRoom {
                    room: 2,
                    load_background: true,
                    unknown_bit_6: false,
                    raw: 0x82,
                }],
                raw_header_hex: "01050b0000000080".into(),
                raw_room_list_hex: "82".into(),
            }],
            cameras: Vec::new(),
            camera_arrows: Vec::new(),
            paths: Vec::new(),
            path_points: Vec::new(),
            scene_transitions: Vec::new(),
            map_events: Vec::new(),
            demo_archive_banks: Vec::new(),
            actor_placements: vec![placement("ACTR", 0, "actor", 0x1234)],
            treasure_placements: Vec::new(),
            player_spawns: Vec::new(),
        },
    );
    let room = archive(
        "files/res/Stage/F_SP00/R02_00.arc",
        "room.dzr",
        digest(4),
        ExtractedStageData {
            chunks: vec![
                ExtractedStageChunk {
                    tag: "PLYR".into(),
                    record_count: 1,
                    data_offset: 64,
                    recognized_record_size: Some(32),
                },
                ExtractedStageChunk {
                    tag: "TREa".into(),
                    record_count: 1,
                    data_offset: 96,
                    recognized_record_size: Some(32),
                },
                ExtractedStageChunk {
                    tag: "SCLS".into(),
                    record_count: 1,
                    data_offset: 128,
                    recognized_record_size: Some(13),
                },
                ExtractedStageChunk {
                    tag: "RCAM".into(),
                    record_count: 1,
                    data_offset: 144,
                    recognized_record_size: Some(24),
                },
                ExtractedStageChunk {
                    tag: "RARO".into(),
                    record_count: 1,
                    data_offset: 168,
                    recognized_record_size: Some(20),
                },
                ExtractedStageChunk {
                    tag: "RPAT".into(),
                    record_count: 1,
                    data_offset: 188,
                    recognized_record_size: Some(12),
                },
                ExtractedStageChunk {
                    tag: "RPPN".into(),
                    record_count: 1,
                    data_offset: 200,
                    recognized_record_size: Some(16),
                },
            ],
            stage_information: None,
            room_transforms: Vec::new(),
            file_lists: Vec::new(),
            room_read_table: Vec::new(),
            cameras: vec![ExtractedCamera {
                record_index: 0,
                camera_type: "FixedFrame".into(),
                arrow_index: 0,
                field_of_view_y: 55,
                argument_0: 2,
                argument_1: 3,
                argument_2: 0xa123,
                camera_type_index: None,
                raw_hex: "46697865644672616d6500000000000000370203a123ffff".into(),
            }],
            camera_arrows: vec![ExtractedCameraArrow {
                record_index: 0,
                position: [10.5, -20.0, 30.25],
                angle: [-1024, 0x4000, 7],
                trailing_i16: -1,
                raw_hex: "41280000c1a0000041f20000fc0040000007ffff".into(),
            }],
            paths: vec![ExtractedPath {
                record_index: 0,
                point_count: 1,
                next_path_index: None,
                path_argument: 4,
                closed: false,
                closed_raw: 0,
                switch_no: None,
                unknown_07: 0xbb,
                point_offset: 0,
                first_point_index: 0,
                raw_hex: "0001ffff0400ffbb00000000".into(),
            }],
            path_points: vec![ExtractedPathPoint {
                record_index: 0,
                arguments: [10, 11, 12, 13],
                position: [7.0, 8.0, 9.0],
                raw_hex: "0b0c0d0a40e000004100000041100000".into(),
            }],
            scene_transitions: vec![ExtractedSceneTransition {
                exit_id: 0,
                destination_stage: "F_SP00".into(),
                destination_spawn: 7,
                destination_room: 2,
                scene_layer: None,
                time_hour: None,
                wipe: 15,
                wipe_time: 7,
                raw_hex: scls_hex,
            }],
            map_events: Vec::new(),
            demo_archive_banks: Vec::new(),
            actor_placements: Vec::new(),
            treasure_placements: vec![placement("TREa", 0, "chest", 9)],
            player_spawns: vec![placement("PLYR", 0, "start", 7)],
        },
    );
    let inventory = build_inventory("F_SP00", vec![&room, &stage]).unwrap();
    assert_eq!(inventory.sources.len(), 2);
    assert_eq!(inventory.sources[0].scope.kind, SourceKind::Stage);
    assert_eq!(inventory.sources[1].scope.room, Some(2));
    assert_eq!(inventory.placements.len(), 2);
    assert_eq!(inventory.placements[1].kind, PlacementKind::Treasure);
    assert_eq!(inventory.player_spawns.len(), 1);
    assert_eq!(inventory.exits[0].raw_field_a, 0xf0);
    assert_eq!(inventory.exits[0].raw_field_b, 0xff);
    assert_eq!(inventory.exits[0].wipe, 0);
    assert!(inventory.collisions.is_empty());
    assert!(inventory.load_triggers.is_empty());
    assert!(inventory.placements[1].stable_id.starts_with("dzr-sha256:"));

    let set = ExtractedOrigWorldInventories {
        schema: EXTRACTED_ORIG_WORLD_INVENTORIES_SCHEMA.into(),
        content_sha256: digest(8),
        game_data_sha256: digest(9),
        source_bundle_sha256: digest(10),
        coverage: expected_coverage(),
        inventories: vec![inventory.clone()],
        stage_metadata: vec![build_stage_metadata("F_SP00", vec![&room, &stage]).unwrap()],
    };
    let bytes = set.canonical_bytes().unwrap();
    assert_eq!(
        ExtractedOrigWorldInventories::decode_canonical(&bytes).unwrap(),
        set
    );
    let mut tampered_path = set.clone();
    tampered_path.stage_metadata[0].paths[0].path.path_argument = 5;
    assert_eq!(
        tampered_path.validate().unwrap_err().field(),
        "orig_world.paths"
    );
    let content = ContentIdentity {
        schema: CONTENT_IDENTITY_SCHEMA.into(),
        id: "gcn-us-native-fixture".into(),
        fingerprint: ContentFingerprint {
            platform: GamePlatform::GameCube,
            region: GameRegion::Usa,
            revision: "fixture".into(),
            product_id: "FIXE01".into(),
            executable_sha256: digest(11),
            game_data_sha256: digest(9),
            resource_manifest_sha256: digest(12),
        },
    };
    let runtime = RuntimeConfiguration {
        schema: RUNTIME_CONFIGURATION_SCHEMA.into(),
        content_sha256: content.digest().unwrap(),
        language: "en".into(),
        settings: BTreeMap::new(),
    };
    let mut import_set = set.clone();
    import_set.content_sha256 = content.digest().unwrap();
    let facts =
        ExtractedWorldFacts::build_from_orig_world_inventories(&content, &runtime, &import_set)
            .unwrap();
    assert_eq!(facts.world_context_sha256, None);
    assert_eq!(
        facts.native_inventory_set_sha256,
        Some(import_set.digest().unwrap())
    );
    assert_eq!(facts.static_world_objects.len(), 3);
    assert_eq!(facts.native_stage_metadata, import_set.stage_metadata);
    assert_eq!(facts.native_stage_metadata[0].room_reads.len(), 1);
    assert_eq!(facts.native_stage_metadata[0].cameras.len(), 1);
    assert_eq!(facts.native_stage_metadata[0].camera_arrows.len(), 1);
    assert_eq!(facts.native_stage_metadata[0].paths.len(), 1);
    assert_eq!(facts.native_stage_metadata[0].path_points.len(), 1);
    assert_eq!(facts.spawns.len(), 1);
    assert_eq!(facts.encoded_exits.len(), 1);
    assert!(
        facts
            .inventories
            .iter()
            .all(|source| source.spatial_index_sha256.is_none())
    );
    let mut mixed_provenance = facts.clone();
    mixed_provenance.world_context_sha256 = Some(digest(13));
    assert!(mixed_provenance.validate().is_err());
    let mut incomplete = set;
    incomplete.inventories[0].player_spawns.clear();
    assert!(incomplete.validate().is_err());
}

#[test]
fn rejects_decoded_fields_that_disagree_with_raw_records() {
    let mut bad = placement("ACTR", 0, "actor", 1);
    bad.parameters = 2;
    assert!(validate_extracted_placement(&bad, PlacementKind::Actor).is_err());
}

#[test]
fn rejects_room_metadata_that_disagrees_with_its_raw_record() {
    let mut raw = [0_u8; 12];
    raw[0..4].copy_from_slice(&10.5_f32.to_bits().to_be_bytes());
    raw[4..8].copy_from_slice(&(-2.0_f32).to_bits().to_be_bytes());
    raw[8..10].copy_from_slice(&0x2000_i16.to_be_bytes());
    raw[10] = 3;
    raw[11] = 0xff;
    let mut metadata = NativeStageMetadata {
        stage: "F_SP00".into(),
        room_transforms: vec![NativeRoomTransformRecord {
            stage: "F_SP00".into(),
            source_sha256: digest(1),
            scope: SourceScope {
                kind: SourceKind::Stage,
                room: None,
            },
            transform: ExtractedRoomTransform {
                record_index: 0,
                room: 3,
                translation_xz: [10.5, -2.0],
                angle_y: 0x2000,
                trailing_byte: 0xff,
                raw_hex: hex_bytes(&raw),
            },
        }],
        file_lists: Vec::new(),
        room_reads: Vec::new(),
        cameras: Vec::new(),
        camera_arrows: Vec::new(),
        paths: Vec::new(),
        path_points: Vec::new(),
    };
    metadata.validate_records().unwrap();
    metadata.room_transforms[0].transform.room = 4;
    assert_eq!(
        metadata.validate_records().unwrap_err().field(),
        "orig_world.room_transforms"
    );
}

#[test]
fn exact_r_sp116_native_inventory_matches_known_record_coverage_when_available() {
    let Some(root) = repository_root() else {
        return;
    };
    let stage_dir = root.join("orig/GZ2E01/files/res/Stage/R_SP116");
    if !stage_dir.is_dir() {
        return;
    }
    let mut archives = Vec::new();
    for (file, resource) in [
        ("STG_00.arc", "stage.dzs"),
        ("R05_00.arc", "room.dzr"),
        ("R06_00.arc", "room.dzr"),
    ] {
        let bytes = fs::read(stage_dir.join(file)).unwrap();
        let resource_bytes = extract_unique_rarc_resource(&bytes, resource).unwrap();
        archives.push(archive(
            &format!("files/res/Stage/R_SP116/{file}"),
            resource,
            Digest(Sha256::digest(&resource_bytes).into()),
            parse_stage_data(&resource_bytes).unwrap(),
        ));
    }
    let refs = archives.iter().collect::<Vec<_>>();
    let inventory = build_inventory("R_SP116", refs).unwrap();
    assert_eq!(inventory.sources.len(), 3);
    assert_eq!(inventory.chunks.len(), 72);
    assert_eq!(inventory.placements.len(), 202);
    assert_eq!(inventory.player_spawns.len(), 14);
    assert_eq!(inventory.exits.len(), 14);
    assert_eq!(
        inventory.sources[2].stage_data_sha256.to_string(),
        "10487ef6754fec1f454c93aa33f605ee9781b4db4b91eed8e864721d76304d40"
    );
}

#[test]
fn exact_native_map_room_metadata_validates_for_every_stage_when_available() {
    let Some(root) = repository_root() else {
        return;
    };
    let stage_root = root.join("orig/GZ2E01/files/res/Stage");
    if !stage_root.is_dir() {
        return;
    }
    let mut stage_dirs = fs::read_dir(&stage_root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    stage_dirs.sort();
    let mut room_reads = 0_usize;
    let mut cameras = 0_usize;
    let mut camera_arrows = 0_usize;
    let mut paths = 0_usize;
    let mut path_points = 0_usize;
    for stage_dir in &stage_dirs {
        let stage = stage_dir.file_name().unwrap().to_str().unwrap();
        let mut archive_paths = fs::read_dir(stage_dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.extension().is_some_and(|extension| extension == "arc")
                    && path.file_name().is_some_and(|name| {
                        let name = name.to_string_lossy();
                        name == "STG_00.arc" || name.starts_with('R') && name.ends_with("_00.arc")
                    })
            })
            .collect::<Vec<_>>();
        archive_paths.sort_by_key(|path| {
            let name = path.file_name().unwrap().to_string_lossy();
            if name == "STG_00.arc" {
                (0, name.into_owned())
            } else {
                (1, name.into_owned())
            }
        });
        let mut archives = Vec::new();
        for path in archive_paths {
            let name = path.file_name().unwrap().to_str().unwrap();
            let resource = if name == "STG_00.arc" {
                "stage.dzs"
            } else {
                "room.dzr"
            };
            let bytes = fs::read(&path).unwrap();
            let resource_bytes = extract_unique_rarc_resource(&bytes, resource).unwrap();
            archives.push(archive(
                &format!("files/res/Stage/{stage}/{name}"),
                resource,
                Digest(Sha256::digest(&resource_bytes).into()),
                parse_stage_data(&resource_bytes).unwrap(),
            ));
            archives.last_mut().unwrap().archive_sha256 = Digest(Sha256::digest(&bytes).into());
        }
        let refs = archives.iter().collect::<Vec<_>>();
        let inventory = build_inventory(stage, refs.clone()).unwrap();
        let metadata = build_stage_metadata(stage, refs).unwrap();
        validate_stage_metadata(&inventory, &metadata).unwrap();
        room_reads += metadata.room_reads.len();
        cameras += metadata.cameras.len();
        camera_arrows += metadata.camera_arrows.len();
        paths += metadata.paths.len();
        path_points += metadata.path_points.len();
    }
    assert_eq!(stage_dirs.len(), 79);
    assert_eq!(room_reads, 1_652);
    assert_eq!(cameras, 1_260);
    assert_eq!(camera_arrows, 1_260);
    assert_eq!(paths, 2_703);
    assert_eq!(path_points, 16_997);
}

fn repository_root() -> Option<PathBuf> {
    let mut path = Path::new(env!("CARGO_MANIFEST_DIR"));
    for _ in 0..5 {
        if path.join("TASKS.md").is_file() {
            return Some(path.to_path_buf());
        }
        path = path.parent()?;
    }
    None
}
