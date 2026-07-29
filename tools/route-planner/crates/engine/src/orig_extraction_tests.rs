use super::*;

fn event_list_fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0x314];
    for (header, top, count) in [
        (0x00, 0x40_u32, 1_i32),
        (0x08, 0xf0, 2),
        (0x10, 0x190, 3),
        (0x18, 0x280, 2),
        (0x20, 0x300, 0),
        (0x28, 0x300, 1),
        (0x30, 0x304, 16),
    ] {
        bytes[header..header + 4].copy_from_slice(&top.to_be_bytes());
        bytes[header + 4..header + 8].copy_from_slice(&count.to_be_bytes());
    }

    let event = &mut bytes[0x40..0xf0];
    event[..10].copy_from_slice(b"demo07_02\0");
    event[0x20..0x24].copy_from_slice(&0_u32.to_be_bytes());
    event[0x28..0x2c].copy_from_slice(&100_i32.to_be_bytes());
    event[0x2c..0x30].copy_from_slice(&0_i32.to_be_bytes());
    event[0x30..0x34].copy_from_slice(&1_i32.to_be_bytes());
    event[0x7c..0x80].copy_from_slice(&2_i32.to_be_bytes());
    for offset in [0x88, 0x8c, 0x90] {
        event[offset..offset + 4].copy_from_slice(&(-1_i32).to_be_bytes());
    }

    for (index, name, staff_type, start_cut) in [
        (0_usize, b"PACKAGE".as_slice(), 11_i32, 0_i32),
        (1, b"DIRECTOR", 6, 2),
    ] {
        let start = 0xf0 + index * 0x50;
        let record = &mut bytes[start..start + 0x50];
        record[..name.len()].copy_from_slice(name);
        record[0x24..0x28].copy_from_slice(&(index as u32).to_be_bytes());
        record[0x2c..0x30].copy_from_slice(&staff_type.to_be_bytes());
        record[0x30..0x34].copy_from_slice(&start_cut.to_be_bytes());
    }

    for (index, name, data_index, next_cut) in [
        (0_usize, "PLAY", 0_i32, 1_i32),
        (1, "WAIT", -1, -1),
        (2, "MAPTOOL", 1, -1),
    ] {
        let start = 0x190 + index * 0x50;
        let record = &mut bytes[start..start + 0x50];
        record[..name.len()].copy_from_slice(name.as_bytes());
        record[0x24..0x28].copy_from_slice(&(index as u32).to_be_bytes());
        for offset in [0x28, 0x2c, 0x30] {
            record[offset..offset + 4].copy_from_slice(&(-1_i32).to_be_bytes());
        }
        record[0x34..0x38].copy_from_slice(&(3_u32 + index as u32).to_be_bytes());
        record[0x38..0x3c].copy_from_slice(&data_index.to_be_bytes());
        record[0x3c..0x40].copy_from_slice(&next_cut.to_be_bytes());
    }

    for (index, name, data_type, value_index, value_count) in [
        (0_usize, "FileName", 4_i32, 0_i32, 16_i32),
        (1, "ID", 3, 0, 1),
    ] {
        let start = 0x280 + index * 0x40;
        let record = &mut bytes[start..start + 0x40];
        record[..name.len()].copy_from_slice(name.as_bytes());
        record[0x20..0x24].copy_from_slice(&(index as u32).to_be_bytes());
        record[0x24..0x28].copy_from_slice(&data_type.to_be_bytes());
        record[0x28..0x2c].copy_from_slice(&value_index.to_be_bytes());
        record[0x2c..0x30].copy_from_slice(&value_count.to_be_bytes());
        record[0x30..0x34].copy_from_slice(&(-1_i32).to_be_bytes());
    }
    bytes[0x300..0x304].copy_from_slice(&4_i32.to_be_bytes());
    bytes[0x304..0x312].copy_from_slice(b"demo07_02.stb\0");
    bytes
}

fn bmg_fixture() -> Vec<u8> {
    let mut bmg = vec![0; 0x20];
    bmg[0..8].copy_from_slice(b"MESGbmg1");
    bmg[12..16].copy_from_slice(&2_u32.to_be_bytes());

    let mut flw1 = vec![0; 0x50];
    flw1[0..4].copy_from_slice(b"FLW1");
    let flw1_size = flw1.len() as u32;
    flw1[4..8].copy_from_slice(&flw1_size.to_be_bytes());
    flw1[8..10].copy_from_slice(&6_u16.to_be_bytes());
    flw1[10..12].copy_from_slice(&2_u16.to_be_bytes());
    flw1[0x10..0x18].copy_from_slice(&[3, 10, 0, 0, 0, 10, 0, 51]);
    flw1[0x18..0x20].copy_from_slice(&[2, 0, 0, 10, 0, 11, 0, 0]);
    flw1[0x20..0x28].copy_from_slice(&[1, 0, 0, 7, 0xff, 0xff, 0, 0]);
    flw1[0x28..0x30].copy_from_slice(&[3, 0, 0, 0, 0, 62, 0, 0]);
    flw1[0x30..0x38].copy_from_slice(&[2, 0, 0, 1, 0, 62, 0, 0]);
    flw1[0x38..0x40].copy_from_slice(&[3, 14, 0, 0, 0, 3, 0, 10]);
    flw1[0x40..0x42].copy_from_slice(&2_u16.to_be_bytes());
    flw1[0x42..0x44].copy_from_slice(&u16::MAX.to_be_bytes());
    bmg.extend(flw1);

    let mut fli1 = vec![0; 0x20];
    fli1[0..4].copy_from_slice(b"FLI1");
    let fli1_size = fli1.len() as u32;
    fli1[4..8].copy_from_slice(&fli1_size.to_be_bytes());
    fli1[8..10].copy_from_slice(&1_u16.to_be_bytes());
    fli1[0x10..0x14].copy_from_slice(&(42_u32 << 16).to_be_bytes());
    fli1[0x14..0x16].copy_from_slice(&0_u16.to_be_bytes());
    bmg.extend(fli1);
    let size = bmg.len() as u32;
    bmg[8..12].copy_from_slice(&size.to_be_bytes());
    bmg
}

#[test]
fn parses_actor_treasure_and_player_placements_without_world_tool_dependencies() {
    let mut stage = vec![0; 0xa0];
    stage[0..4].copy_from_slice(&3_u32.to_be_bytes());
    for (index, (tag, offset)) in [
        (b"ACT5", 0x40_u32),
        (b"PLYR", 0x60_u32),
        (b"TREa", 0x80_u32),
    ]
    .into_iter()
    .enumerate()
    {
        let header = 4 + index * 12;
        stage[header..header + 4].copy_from_slice(tag);
        stage[header + 4..header + 8].copy_from_slice(&1_u32.to_be_bytes());
        stage[header + 8..header + 12].copy_from_slice(&offset.to_be_bytes());
    }
    stage[0x40..0x44].copy_from_slice(b"grD1");
    stage[0x48..0x4c].copy_from_slice(&0x12345678_u32.to_be_bytes());
    stage[0x4c..0x50].copy_from_slice(&1.5_f32.to_bits().to_be_bytes());
    stage[0x50..0x54].copy_from_slice(&(-2.0_f32).to_bits().to_be_bytes());
    stage[0x54..0x58].copy_from_slice(&3.25_f32.to_bits().to_be_bytes());
    stage[0x58..0x5a].copy_from_slice(&42_i16.to_be_bytes());
    stage[0x5a..0x5c].copy_from_slice(&(-1_i16).to_be_bytes());
    stage[0x5c..0x5e].copy_from_slice(&9_i16.to_be_bytes());
    stage[0x5e..0x60].copy_from_slice(&7_u16.to_be_bytes());
    stage[0x60..0x65].copy_from_slice(b"start");
    stage[0x80..0x85].copy_from_slice(b"Tbox0");
    stage[0x88..0x8c].copy_from_slice(&0xfeed_beef_u32.to_be_bytes());

    let parsed = parse_stage_data(&stage).unwrap();
    assert_eq!(parsed.chunks[0].recognized_record_size, Some(0x20));
    let actor = &parsed.actor_placements[0];
    assert_eq!(actor.name, "grD1");
    assert_eq!(actor.layer, Some(5));
    assert_eq!(actor.parameters, 0x12345678);
    assert_eq!(actor.position, [1.5, -2.0, 3.25]);
    assert_eq!(actor.angle, [42, -1, 9]);
    assert_eq!(actor.set_id, 7);
    assert_eq!(parsed.player_spawns.len(), 1);
    assert_eq!(parsed.player_spawns[0].name, "start");
    assert_eq!(parsed.player_spawns[0].layer, None);
    assert_eq!(parsed.treasure_placements.len(), 1);
    assert_eq!(parsed.treasure_placements[0].name, "Tbox0");
    assert_eq!(parsed.treasure_placements[0].parameters, 0xfeed_beef);
    assert_eq!(parsed.treasure_placements[0].layer, Some(10));
}

#[test]
fn real_rsp116_room6_placement_counts_match_the_compatible_inventory_when_present() {
    use sha2::{Digest as _, Sha256};
    use std::path::Path;

    let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(4)
        .unwrap();
    let archive_path = repository_root.join("orig/GZ2E01/files/res/Stage/R_SP116/R06_00.arc");
    if !archive_path.is_file() {
        return;
    }
    let archive = std::fs::read(archive_path).unwrap();
    let resource = extract_unique_rarc_resource(&archive, "room.dzr").unwrap();
    assert_eq!(
        hex_bytes(&Sha256::digest(&resource)),
        "10487ef6754fec1f454c93aa33f605ee9781b4db4b91eed8e864721d76304d40"
    );
    let parsed = parse_stage_data(&resource).unwrap();
    assert_eq!(parsed.actor_placements.len(), 95);
    assert_eq!(parsed.player_spawns.len(), 5);
    assert_eq!(parsed.treasure_placements.len(), 0);
    assert_eq!(
        parsed
            .chunks
            .iter()
            .find(|chunk| chunk.tag == "PLYR")
            .map(|chunk| (chunk.record_count, chunk.recognized_record_size)),
        Some((5, Some(0x20)))
    );
}

#[test]
fn real_stage_room_read_tables_cover_every_stage_when_present() {
    use std::path::Path;

    let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(4)
        .unwrap();
    let stage_root = repository_root.join("orig/GZ2E01/files/res/Stage");
    if !stage_root.is_dir() {
        return;
    }
    let mut archives = std::fs::read_dir(stage_root)
        .unwrap()
        .map(|entry| entry.unwrap().path().join("STG_00.arc"))
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    archives.sort();
    let mut room_reads = 0_usize;
    for archive_path in &archives {
        let archive = std::fs::read(archive_path).unwrap();
        let resource = extract_unique_rarc_resource(&archive, "stage.dzs").unwrap();
        let parsed = parse_stage_data(&resource).unwrap();
        let declared = parsed
            .chunks
            .iter()
            .find(|chunk| chunk.tag == "RTBL")
            .unwrap()
            .record_count as usize;
        assert_eq!(parsed.room_read_table.len(), declared);
        room_reads += declared;
    }
    assert_eq!(archives.len(), 79);
    assert_eq!(room_reads, 1_652);
}

#[test]
fn parses_stage_message_group_and_indexed_scene_transitions() {
    let mut stage = vec![0; 0xa9];
    stage[0..4].copy_from_slice(&2_u32.to_be_bytes());
    stage[4..8].copy_from_slice(b"STAG");
    stage[8..12].copy_from_slice(&1_u32.to_be_bytes());
    stage[12..16].copy_from_slice(&0x20_u32.to_be_bytes());
    stage[16..20].copy_from_slice(b"SCLS");
    stage[20..24].copy_from_slice(&2_u32.to_be_bytes());
    stage[24..28].copy_from_slice(&0x8f_u32.to_be_bytes());
    stage[0x48] = 3;

    let first = &mut stage[0x8f..0x9c];
    first[..8].copy_from_slice(b"D_MN04\0\0");
    first[8..13].copy_from_slice(&[1, 1, 0xf0, 0x1f, 0]);
    let second = &mut stage[0x9c..0xa9];
    second[..8].copy_from_slice(b"F_SP110\0");
    second[8..13].copy_from_slice(&[3, 0, 0x70, 0x42, 13]);

    let parsed = parse_stage_data(&stage).unwrap();
    assert_eq!(parsed.stage_information.unwrap().message_group, 3);
    assert_eq!(parsed.scene_transitions.len(), 2);
    assert_eq!(
        parsed.scene_transitions[0],
        ExtractedSceneTransition {
            exit_id: 0,
            destination_stage: "D_MN04".into(),
            destination_spawn: 1,
            destination_room: 1,
            scene_layer: None,
            time_hour: None,
            wipe: 0,
            wipe_time: 0,
            raw_hex: "445f4d4e303400000101f01f00".into(),
        }
    );
    assert_eq!(parsed.scene_transitions[1].exit_id, 1);
    assert_eq!(parsed.scene_transitions[1].scene_layer, Some(2));
    assert_eq!(parsed.scene_transitions[1].time_hour, Some(7));
    assert_eq!(parsed.scene_transitions[1].wipe_time, 2);
}

#[test]
fn parses_room_background_transforms_and_normal_file_lists() {
    let mut stage = vec![0; 0x60];
    stage[..4].copy_from_slice(&2_u32.to_be_bytes());
    stage[4..8].copy_from_slice(b"MULT");
    stage[8..12].copy_from_slice(&1_u32.to_be_bytes());
    stage[12..16].copy_from_slice(&0x20_u32.to_be_bytes());
    stage[16..20].copy_from_slice(b"FILI");
    stage[20..24].copy_from_slice(&1_u32.to_be_bytes());
    stage[24..28].copy_from_slice(&0x40_u32.to_be_bytes());

    let transform = &mut stage[0x20..0x2c];
    transform[0..4].copy_from_slice(&125.5_f32.to_bits().to_be_bytes());
    transform[4..8].copy_from_slice(&(-42.25_f32).to_bits().to_be_bytes());
    transform[8..10].copy_from_slice(&0x4000_i16.to_be_bytes());
    transform[10] = 7;
    transform[11] = 0xaa;

    let file_list = &mut stage[0x40..0x60];
    let parameters = 0x2000_0000_u32 | (2 << 18) | (5 << 15) | (0x34 << 7) | (3 << 3);
    file_list[0..4].copy_from_slice(&parameters.to_be_bytes());
    file_list[4..8].copy_from_slice(&(-100.0_f32).to_bits().to_be_bytes());
    file_list[8..12].copy_from_slice(&1.25_f32.to_bits().to_be_bytes());
    file_list[12..16].copy_from_slice(&2.5_f32.to_bits().to_be_bytes());
    file_list[16..26].copy_from_slice(&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
    file_list[0x1a] = 4;
    file_list[0x1b] = 0xff;
    file_list[0x1c..0x1e].copy_from_slice(&123_u16.to_be_bytes());

    let parsed = parse_stage_data(&stage).unwrap();
    assert_eq!(parsed.chunks[0].recognized_record_size, Some(0x0c));
    assert_eq!(parsed.chunks[1].recognized_record_size, Some(0x20));
    assert_eq!(parsed.room_transforms[0].room, 7);
    assert_eq!(parsed.room_transforms[0].translation_xz, [125.5, -42.25]);
    assert_eq!(parsed.room_transforms[0].angle_y, 0x4000);
    assert_eq!(parsed.room_transforms[0].trailing_byte, 0xaa);
    let fili = &parsed.file_lists[0];
    assert_eq!(fili.sea_level, -100.0);
    assert_eq!(fili.minimap_style, 3);
    assert!(fili.enemy_appear_flag);
    assert_eq!(fili.global_wind_level, 2);
    assert_eq!(fili.global_wind_direction, 5);
    assert_eq!(fili.grass_light, 0x34);
    assert_eq!(fili.default_camera, 4);
    assert_eq!(fili.bit_switch, 0xff);
    assert_eq!(fili.message_id, 123);
}

#[test]
fn parses_room_camera_and_arrow_records_without_inventing_arguments() {
    let mut stage = vec![0; 0x54];
    stage[..4].copy_from_slice(&2_u32.to_be_bytes());
    stage[4..8].copy_from_slice(b"RCAM");
    stage[8..12].copy_from_slice(&1_u32.to_be_bytes());
    stage[12..16].copy_from_slice(&0x20_u32.to_be_bytes());
    stage[16..20].copy_from_slice(b"RARO");
    stage[20..24].copy_from_slice(&1_u32.to_be_bytes());
    stage[24..28].copy_from_slice(&0x40_u32.to_be_bytes());

    let camera = &mut stage[0x20..0x38];
    camera[..10].copy_from_slice(b"FixedFrame");
    camera[0x10..0x14].copy_from_slice(&[0, 55, 2, 3]);
    camera[0x14..0x16].copy_from_slice(&0xa123_u16.to_be_bytes());
    camera[0x16..0x18].copy_from_slice(&u16::MAX.to_be_bytes());

    let arrow = &mut stage[0x40..0x54];
    arrow[0..4].copy_from_slice(&10.5_f32.to_bits().to_be_bytes());
    arrow[4..8].copy_from_slice(&(-20.0_f32).to_bits().to_be_bytes());
    arrow[8..12].copy_from_slice(&30.25_f32.to_bits().to_be_bytes());
    arrow[0x0c..0x0e].copy_from_slice(&(-1024_i16).to_be_bytes());
    arrow[0x0e..0x10].copy_from_slice(&0x4000_i16.to_be_bytes());
    arrow[0x10..0x12].copy_from_slice(&7_i16.to_be_bytes());
    arrow[0x12..0x14].copy_from_slice(&(-1_i16).to_be_bytes());

    let parsed = parse_stage_data(&stage).unwrap();
    assert_eq!(parsed.chunks[0].recognized_record_size, Some(0x18));
    assert_eq!(parsed.chunks[1].recognized_record_size, Some(0x14));
    assert_eq!(parsed.cameras.len(), 1);
    assert_eq!(parsed.cameras[0].camera_type, "FixedFrame");
    assert_eq!(parsed.cameras[0].arrow_index, 0);
    assert_eq!(parsed.cameras[0].field_of_view_y, 55);
    assert_eq!(parsed.cameras[0].argument_0, 2);
    assert_eq!(parsed.cameras[0].argument_1, 3);
    assert_eq!(parsed.cameras[0].argument_2, 0xa123);
    assert_eq!(parsed.cameras[0].camera_type_index, None);
    assert_eq!(parsed.camera_arrows[0].position, [10.5, -20.0, 30.25]);
    assert_eq!(parsed.camera_arrows[0].angle, [-1024, 0x4000, 7]);
    assert_eq!(parsed.camera_arrows[0].trailing_i16, -1);
}

#[test]
fn parses_path_graphs_and_normalizes_point_spans() {
    let mut stage = vec![0; 0x70];
    stage[..4].copy_from_slice(&2_u32.to_be_bytes());
    stage[4..8].copy_from_slice(b"RPAT");
    stage[8..12].copy_from_slice(&2_u32.to_be_bytes());
    stage[12..16].copy_from_slice(&0x20_u32.to_be_bytes());
    stage[16..20].copy_from_slice(b"RPPN");
    stage[20..24].copy_from_slice(&3_u32.to_be_bytes());
    stage[24..28].copy_from_slice(&0x40_u32.to_be_bytes());

    stage[0x20..0x22].copy_from_slice(&2_u16.to_be_bytes());
    stage[0x22..0x24].copy_from_slice(&1_u16.to_be_bytes());
    stage[0x24..0x28].copy_from_slice(&[9, 0x81, 7, 0xaa]);
    stage[0x28..0x2c].copy_from_slice(&0_u32.to_be_bytes());
    stage[0x2c..0x2e].copy_from_slice(&1_u16.to_be_bytes());
    stage[0x2e..0x30].copy_from_slice(&u16::MAX.to_be_bytes());
    stage[0x30..0x34].copy_from_slice(&[4, 0, 0xff, 0xbb]);
    stage[0x34..0x38].copy_from_slice(&0x20_u32.to_be_bytes());

    for (index, (arguments, position)) in [
        ([10_u8, 11, 12, 13], [1.0_f32, 2.0, 3.0]),
        ([20, 21, 22, 23], [4.0, 5.0, 6.0]),
        ([30, 31, 32, 33], [7.0, 8.0, 9.0]),
    ]
    .into_iter()
    .enumerate()
    {
        let offset = 0x40 + index * 0x10;
        // On disc the point arguments are ordered arg1, arg2, arg3, arg0.
        stage[offset..offset + 4].copy_from_slice(&[
            arguments[1],
            arguments[2],
            arguments[3],
            arguments[0],
        ]);
        for (axis, value) in position.into_iter().enumerate() {
            stage[offset + 4 + axis * 4..offset + 8 + axis * 4]
                .copy_from_slice(&value.to_bits().to_be_bytes());
        }
    }

    let parsed = parse_stage_data(&stage).unwrap();
    assert_eq!(parsed.paths.len(), 2);
    assert_eq!(parsed.path_points.len(), 3);
    assert_eq!(parsed.paths[0].point_count, 2);
    assert_eq!(parsed.paths[0].next_path_index, Some(1));
    assert_eq!(parsed.paths[0].path_argument, 9);
    assert!(parsed.paths[0].closed);
    assert_eq!(parsed.paths[0].closed_raw, 0x81);
    assert_eq!(parsed.paths[0].switch_no, Some(7));
    assert_eq!(parsed.paths[0].first_point_index, 0);
    assert_eq!(parsed.paths[1].next_path_index, None);
    assert_eq!(parsed.paths[1].switch_no, None);
    assert_eq!(parsed.paths[1].first_point_index, 2);
    assert_eq!(parsed.path_points[0].arguments, [10, 11, 12, 13]);
    assert_eq!(parsed.path_points[2].position, [7.0, 8.0, 9.0]);

    let mut bad_next = stage.clone();
    bad_next[0x22..0x24].copy_from_slice(&2_u16.to_be_bytes());
    assert_eq!(
        parse_stage_data(&bad_next).unwrap_err().field(),
        "orig.stage.rpat"
    );
    stage[0x34..0x38].copy_from_slice(&0x30_u32.to_be_bytes());
    assert_eq!(
        parse_stage_data(&stage).unwrap_err().field(),
        "orig.stage.rpat"
    );
}

#[test]
fn parses_pointer_backed_room_read_topology() {
    let mut stage = vec![0; 0x2b];
    stage[..4].copy_from_slice(&1_u32.to_be_bytes());
    stage[4..8].copy_from_slice(b"RTBL");
    stage[8..12].copy_from_slice(&2_u32.to_be_bytes());
    stage[12..16].copy_from_slice(&0x10_u32.to_be_bytes());
    stage[0x10..0x14].copy_from_slice(&0x18_u32.to_be_bytes());
    stage[0x14..0x18].copy_from_slice(&0x20_u32.to_be_bytes());
    stage[0x18..0x20].copy_from_slice(&[2, 0x85, 0x0b, 0xaa, 0, 0, 0, 0x28]);
    stage[0x20..0x28].copy_from_slice(&[1, 2, 0, 0, 0, 0, 0, 0x2a]);
    stage[0x28..0x2b].copy_from_slice(&[0x81, 0x42, 0x03]);

    let parsed = parse_stage_data(&stage).unwrap();
    assert_eq!(parsed.chunks[0].recognized_record_size, None);
    assert_eq!(parsed.room_read_table.len(), 2);
    let room0 = &parsed.room_read_table[0];
    assert_eq!(room0.room_index, 0);
    assert_eq!(room0.reverb, 5);
    assert_eq!(room0.reverb_raw, 0x85);
    assert_eq!(room0.time_pass, 3);
    assert!(room0.vrbox_enabled);
    assert_eq!(room0.padding, 0xaa);
    assert_eq!(room0.load_rooms[0].room, 1);
    assert!(room0.load_rooms[0].load_background);
    assert_eq!(room0.load_rooms[1].room, 2);
    assert!(room0.load_rooms[1].unknown_bit_6);
    assert_eq!(parsed.room_read_table[1].load_rooms[0].room, 3);
}

#[test]
fn parses_demo_banks_and_map_event_exit_coordinates() {
    let mut stage = vec![0; 0x70];
    stage[..4].copy_from_slice(&2_u32.to_be_bytes());
    stage[4..8].copy_from_slice(b"REVT");
    stage[8..12].copy_from_slice(&1_u32.to_be_bytes());
    stage[12..16].copy_from_slice(&0x20_u32.to_be_bytes());
    stage[16..20].copy_from_slice(b"LBNK");
    stage[20..24].copy_from_slice(&15_u32.to_be_bytes());
    stage[24..28].copy_from_slice(&0x40_u32.to_be_bytes());
    let map_event = &mut stage[0x20..0x3c];
    map_event[..13].copy_from_slice(&[2, 2, 3, 3, 4, 0xff, 100, 1, 3, 2, 0xff, 0xff, 0]);
    map_event[13..23].copy_from_slice(b"demo07_02\0");
    map_event[0x1a] = 0xff;
    map_event[0x1b] = 0xff;
    stage[0x40..0x40 + 45].fill(0xff);
    stage[0x40 + 8 * 3..0x40 + 8 * 3 + 3].copy_from_slice(&[7, 2, 0xff]);

    let parsed = parse_stage_data(&stage).unwrap();
    assert_eq!(parsed.chunks[0].recognized_record_size, Some(0x1c));
    assert_eq!(parsed.map_events[0].map_tool_id, 4);
    assert_eq!(parsed.map_events[0].normal_exit_id, Some(1));
    assert_eq!(parsed.map_events[0].skip_exit_id, Some(2));
    assert_eq!(
        parsed.map_events[0].event_name.as_deref(),
        Some("demo07_02")
    );
    assert_eq!(
        parsed.demo_archive_banks[8].archive_name.as_deref(),
        Some("Demo07_02")
    );
}

#[test]
fn parses_event_staff_cut_and_typed_data_tables() {
    let parsed = parse_event_list(&event_list_fixture()).unwrap();
    assert_eq!(parsed.events[0].name, "demo07_02");
    assert_eq!(parsed.events[0].staff_indices, [0, 1]);
    assert_eq!(parsed.staff[0].name, "PACKAGE");
    assert_eq!(parsed.staff[0].start_cut_index, 0);
    assert_eq!(parsed.cuts[0].name, "PLAY");
    assert_eq!(parsed.cuts[0].data_index, Some(0));
    assert_eq!(parsed.cuts[0].next_cut_index, Some(1));
    assert_eq!(
        parsed.data[0].value,
        ExtractedEventDataValue::StringBytes {
            raw_hex: "64656d6f30375f30322e737462000000".into(),
            ascii: Some("demo07_02.stb".into()),
        }
    );
    assert_eq!(
        parsed.data[1].value,
        ExtractedEventDataValue::Integers { values: vec![4] }
    );

    let mut invalid = event_list_fixture();
    invalid[0xf0 + 0x30..0xf0 + 0x34].copy_from_slice(&99_i32.to_be_bytes());
    assert_eq!(
        parse_event_list(&invalid).unwrap_err().field(),
        "orig.event_list.staff.start_cut"
    );
}

#[test]
fn parses_flow_labels_generic_temp_writers_and_reader() {
    let parsed = parse_message_flow(&bmg_fixture()).unwrap();
    assert_eq!(parsed.node_count, 6);
    assert_eq!(parsed.labels[0].flow_id, 42);
    assert_eq!(parsed.branch_targets, vec![2, u16::MAX]);
    assert_eq!(
        parsed.nodes[0],
        MessageFlowNode::Event {
            index: 0,
            event_index: 10,
            next_target_index: 0,
            parameter_0: 10,
            parameter_1: 51,
            raw_parameter_u32: 0x000a0033,
            raw_parameters: [0, 10, 0, 51],
        }
    );
    assert_eq!(
        parsed.nodes[1],
        MessageFlowNode::Branch {
            index: 1,
            flags: 0,
            raw_query_index: 10,
            query_handler_index: Some(11),
            parameter: 11,
            next_target_index: 0,
        }
    );
    assert_eq!(
        parsed.temporary_flag_accesses,
        vec![
            MessageFlowTemporaryFlagAccess {
                node_index: 0,
                operation: MessageFlowTemporaryFlagOperation::Set,
                parameter_ordinal: 0,
                label_index: 10,
                packed_backing_coordinate: None,
                friendly_name: None,
            },
            MessageFlowTemporaryFlagAccess {
                node_index: 0,
                operation: MessageFlowTemporaryFlagOperation::Set,
                parameter_ordinal: 1,
                label_index: 51,
                packed_backing_coordinate: Some(0x0508),
                friendly_name: Some("message_flow_control_f".to_owned()),
            },
            MessageFlowTemporaryFlagAccess {
                node_index: 1,
                operation: MessageFlowTemporaryFlagOperation::BranchTrueWhenClear,
                parameter_ordinal: 0,
                label_index: 11,
                packed_backing_coordinate: Some(0x0004),
                friendly_name: Some("message_flow_control_a".to_owned()),
            },
        ]
    );
    assert_eq!(
        parsed.persistent_flag_accesses,
        vec![
            MessageFlowPersistentFlagAccess {
                node_index: 3,
                operation: MessageFlowPersistentFlagOperation::Set,
                parameter_ordinal: 0,
                label_index: 62,
                packed_backing_coordinate: Some(0x0704),
                friendly_name: Some("won_gor_coron_match".to_owned()),
            },
            MessageFlowPersistentFlagAccess {
                node_index: 4,
                operation: MessageFlowPersistentFlagOperation::BranchTrueWhenClear,
                parameter_ordinal: 0,
                label_index: 62,
                packed_backing_coordinate: Some(0x0704),
                friendly_name: Some("won_gor_coron_match".to_owned()),
            },
        ]
    );
    assert_eq!(
        parsed.switch_accesses,
        vec![MessageFlowSwitchAccess {
            node_index: 5,
            operation: MessageFlowSwitchOperation::Set,
            store: MessageFlowSwitchStore::OneZone,
            switch_index: 10,
        }]
    );
}

#[test]
fn resolves_audited_lanayru_persistent_message_labels_to_raw_backing() {
    assert_eq!(
        persistent_message_flag(615),
        Some((0x4b04, "received_lanayru_vessel"))
    );
    assert_eq!(
        persistent_message_flag(66),
        Some((0x0840, "start_carriage_guarding_game"))
    );
}

#[test]
fn accepts_retail_header_and_final_alignment_quirks_without_ignoring_payload() {
    let mut fixture = bmg_fixture();
    let fli_offset = 0x20 + 0x50;
    fixture[8..12].copy_from_slice(&(fli_offset as u32).to_be_bytes());
    fixture[fli_offset + 4..fli_offset + 8].copy_from_slice(&(0x28_u32).to_be_bytes());

    let parsed = parse_message_flow(&fixture).unwrap();
    assert_eq!(parsed.header_declared_size, fli_offset as u32);
    assert_eq!(parsed.resource_size, fixture.len() as u32);
    assert_eq!(parsed.labels[0].flow_id, 42);
}

#[test]
fn maps_raw_query_dispatch_indices_without_conflating_handler_numbers() {
    assert_eq!(message_query_handler_index(0), Some(5));
    assert_eq!(message_query_handler_index(6), Some(4));
    assert_eq!(message_query_handler_index(10), Some(11));
    assert_eq!(message_query_handler_index(52), Some(53));
    assert_eq!(message_query_handler_index(53), None);
}

#[test]
fn malformed_or_oversized_inputs_fail_closed() {
    assert!(parse_message_flow(b"MESGbmg1").is_err());
    assert!(extract_unique_rarc_resource(b"RARC", "zel_04.bmg").is_err());
    let mut yaz0 = vec![0; 16];
    yaz0[0..4].copy_from_slice(b"Yaz0");
    yaz0[4..8].copy_from_slice(&((MAX_DECODED_ARCHIVE_BYTES as u32) + 1).to_be_bytes());
    assert!(extract_unique_rarc_resource(&yaz0, "zel_04.bmg").is_err());
}
