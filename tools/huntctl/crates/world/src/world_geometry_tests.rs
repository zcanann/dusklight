
use super::*;

fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}

fn put_f32(bytes: &mut [u8], offset: usize, value: f32) {
    put_u32(bytes, offset, value.to_bits());
}

fn put_vec3(bytes: &mut [u8], offset: usize, value: Vec3) {
    put_f32(bytes, offset, value.x);
    put_f32(bytes, offset + 4, value.y);
    put_f32(bytes, offset + 8, value.z);
}

fn fixture() -> (Vec<u8>, Vec<u8>) {
    let position_offset = KCL_HEADER_SIZE;
    let normal_offset = position_offset + 12;
    let prism_offset = normal_offset + 4 * 12;
    let block_offset = prism_offset + 2 * KCL_PRISM_SIZE;
    let mut kcl = vec![0_u8; block_offset + 4];
    put_u32(&mut kcl, 0, position_offset as u32);
    put_u32(&mut kcl, 4, normal_offset as u32);
    put_u32(&mut kcl, 8, prism_offset as u32);
    put_u32(&mut kcl, 12, block_offset as u32);
    put_vec3(
        &mut kcl,
        position_offset,
        Vec3 {
            x: 10.0,
            y: 20.0,
            z: 30.0,
        },
    );
    for (index, normal) in [
        Vec3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        },
        Vec3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        },
        Vec3 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
        },
        Vec3 {
            x: 1.0,
            y: 0.0,
            z: 1.0,
        },
    ]
    .into_iter()
    .enumerate()
    {
        put_vec3(&mut kcl, normal_offset + index * 12, normal);
    }
    let prism = prism_offset + KCL_PRISM_SIZE;
    put_f32(&mut kcl, prism, 1.0);
    for (offset, value) in [(4, 0), (6, 0), (8, 1), (10, 2), (12, 3), (14, 0)] {
        put_u16(&mut kcl, prism + offset, value);
    }

    let mut plc = vec![0_u8; PLC_HEADER_SIZE + PLC_CODE_SIZE];
    plc[0..4].copy_from_slice(b"SPLC");
    put_u16(&mut plc, 4, PLC_CODE_SIZE as u16);
    put_u16(&mut plc, 6, 1);
    for (index, word) in [
        1 | (7 << 6) | (8 << 24),
        9 | (2 << 8) | (3 << 12) | (4 << 16) | (5 << 19),
        6 | (7 << 8) | (8 << 16) | (9 << 24),
        0x1234_5678,
        10 | (11 << 11) | (12 << 20),
    ]
    .into_iter()
    .enumerate()
    {
        put_u32(&mut plc, PLC_HEADER_SIZE + index * 4, word);
    }
    (kcl, plc)
}

#[test]
fn inspects_content_addressed_prism_geometry_and_raw_code() {
    let (kcl, plc) = fixture();
    let inspection = KclPlc::parse(&kcl, &plc).unwrap().inspect_prism(1).unwrap();
    assert_eq!(inspection.position_count, 1);
    assert_eq!(inspection.normal_count, 4);
    assert_eq!(inspection.prism_table_count, 2);
    assert_eq!(inspection.plc_code_count, 1);
    assert_eq!(inspection.kcl_sha256, sha256(&kcl));
    assert_eq!(inspection.plc_sha256, sha256(&plc));
    assert_eq!(inspection.prism.attribute, 0);
    assert_eq!(inspection.prism.code.exit_id, 1);
    assert_eq!(inspection.prism.code.raw[3], 0x1234_5678);
    assert_eq!(inspection.prism.code.ground_code, 5);
    assert_eq!(inspection.prism.code.room, 12);
    assert_eq!(inspection.prism.plane.d, -20.0);
    assert_eq!(
        inspection.prism.triangle,
        [
            Vec3 {
                x: 10.0,
                y: 20.0,
                z: 30.0
            },
            Vec3 {
                x: 10.0,
                y: 20.0,
                z: 31.0
            },
            Vec3 {
                x: 11.0,
                y: 20.0,
                z: 30.0
            },
        ]
    );
    assert!(inspection.prism.stable_id.starts_with("kcl-sha256:"));
}

#[test]
fn measures_point_to_prism_plane_and_triangle() {
    let (kcl, plc) = fixture();
    let prism = KclPlc::parse(&kcl, &plc)
        .unwrap()
        .inspect_prism(1)
        .unwrap()
        .prism;
    let query = query_prism_point(
        &prism,
        Vec3 {
            x: 10.25,
            y: 22.0,
            z: 30.25,
        },
    )
    .unwrap();
    assert_eq!(query.signed_plane_distance, 2.0);
    assert_eq!(
        query.closest_point,
        Vec3 {
            x: 10.25,
            y: 20.0,
            z: 30.25,
        }
    );
    assert_eq!(query.distance, 2.0);

    let outside = query_prism_point(
        &prism,
        Vec3 {
            x: 12.0,
            y: 22.0,
            z: 32.0,
        },
    )
    .unwrap();
    assert_eq!(
        outside.closest_point,
        Vec3 {
            x: 10.5,
            y: 20.0,
            z: 30.5,
        }
    );
    assert!((outside.distance - 8.5_f64.sqrt()).abs() < 1.0e-6);
}

#[test]
fn closest_point_falls_back_for_degenerate_triangle() {
    let point = Vec3d {
        x: 1.0,
        y: 2.0,
        z: 0.0,
    };
    let closest = closest_point_on_triangle(
        point,
        [
            Vec3d {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            Vec3d {
                x: 2.0,
                y: 0.0,
                z: 0.0,
            },
            Vec3d {
                x: 4.0,
                y: 0.0,
                z: 0.0,
            },
        ],
    );
    assert_eq!(closest.x, 1.0);
    assert_eq!(closest.y, 0.0);
    assert_eq!(closest.z, 0.0);
}

#[test]
fn rejects_reserved_out_of_range_and_cross_file_indices() {
    let (kcl, plc) = fixture();
    let parsed = KclPlc::parse(&kcl, &plc).unwrap();
    assert!(
        parsed
            .inspect_prism(0)
            .unwrap_err()
            .to_string()
            .contains("reserved")
    );
    assert!(
        parsed
            .inspect_prism(2)
            .unwrap_err()
            .to_string()
            .contains("outside table")
    );

    let mut invalid_attribute = kcl;
    let prism_offset = read_u32(&invalid_attribute, 8, "test").unwrap() as usize;
    put_u16(
        &mut invalid_attribute,
        prism_offset + KCL_PRISM_SIZE + 14,
        1,
    );
    assert!(
        KclPlc::parse(&invalid_attribute, &plc)
            .unwrap()
            .inspect_prism(1)
            .unwrap_err()
            .to_string()
            .contains("outside PLC")
    );
}

#[test]
fn rejects_degenerate_geometry_and_invalid_table_order() {
    let (mut kcl, plc) = fixture();
    let normal_offset = read_u32(&kcl, 4, "test").unwrap() as usize;
    put_vec3(
        &mut kcl,
        normal_offset + 3 * 12,
        Vec3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        },
    );
    assert!(
        KclPlc::parse(&kcl, &plc)
            .unwrap()
            .inspect_prism(1)
            .unwrap_err()
            .to_string()
            .contains("degenerate")
    );

    put_u32(&mut kcl, 8, (normal_offset - 1) as u32);
    assert!(KclPlc::parse(&kcl, &plc).is_err());
}

fn rarc_with(name: &str, resource: &[u8]) -> Vec<u8> {
    let file_table = 0x40_usize;
    let string_table = 0x60_usize;
    let data = 0x80_usize;
    let mut archive = vec![0_u8; data + resource.len()];
    archive[0..4].copy_from_slice(b"RARC");
    put_u32(&mut archive, 12, (data - 0x20) as u32);
    put_u32(&mut archive, 0x28, 1);
    put_u32(&mut archive, 0x2c, (file_table - 0x20) as u32);
    put_u32(&mut archive, 0x34, (string_table - 0x20) as u32);
    put_u16(&mut archive, file_table + 4, 0x0100);
    put_u32(&mut archive, file_table + 8, 0);
    put_u32(&mut archive, file_table + 12, resource.len() as u32);
    archive[string_table..string_table + name.len()].copy_from_slice(name.as_bytes());
    archive[string_table + name.len()] = 0;
    archive[data..].copy_from_slice(resource);
    archive
}

fn indexed_rarc_with(name: &str, resource: &[u8]) -> Vec<u8> {
    let node_table = 0x40_usize;
    let file_table = node_table + RARC_NODE_SIZE;
    let string_table = file_table + RARC_FILE_ENTRY_SIZE;
    let root_name = b"root";
    let string_size = root_name.len() + 1 + name.len() + 1;
    let data = (string_table + string_size + 0x1f) & !0x1f;
    let mut archive = vec![0_u8; data + resource.len()];
    archive[0..4].copy_from_slice(b"RARC");
    let archive_len = archive.len() as u32;
    put_u32(&mut archive, 4, archive_len);
    put_u32(&mut archive, 12, (data - 0x20) as u32);
    put_u32(&mut archive, 0x20, 1);
    put_u32(&mut archive, 0x24, (node_table - 0x20) as u32);
    put_u32(&mut archive, 0x28, 1);
    put_u32(&mut archive, 0x2c, (file_table - 0x20) as u32);
    put_u32(&mut archive, 0x30, string_size as u32);
    put_u32(&mut archive, 0x34, (string_table - 0x20) as u32);
    archive[node_table..node_table + 4].copy_from_slice(b"ROOT");
    put_u32(&mut archive, node_table + 4, 0);
    put_u16(&mut archive, node_table + 10, 1);
    put_u32(&mut archive, node_table + 12, 0);
    put_u16(&mut archive, file_table + 4, 0x0100);
    put_u16(&mut archive, file_table + 6, (root_name.len() + 1) as u16);
    put_u32(&mut archive, file_table + 8, 0);
    put_u32(&mut archive, file_table + 12, resource.len() as u32);
    archive[string_table..string_table + root_name.len()].copy_from_slice(root_name);
    let name_start = string_table + root_name.len() + 1;
    archive[name_start..name_start + name.len()].copy_from_slice(name.as_bytes());
    archive[data..].copy_from_slice(resource);
    archive
}

fn yaz0_literals(source: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(16 + source.len() + source.len().div_ceil(8));
    output.extend_from_slice(b"Yaz0");
    output.extend_from_slice(&(source.len() as u32).to_be_bytes());
    output.extend_from_slice(&[0; 8]);
    for chunk in source.chunks(8) {
        output.push(0xff);
        output.extend_from_slice(chunk);
    }
    output
}

#[test]
fn extracts_named_resource_from_rarc_and_yaz0_without_mutating_input() {
    let rarc = rarc_with("room.kcl", b"immutable collision bytes");
    let original = rarc.clone();
    assert_eq!(
        extract_rarc_resource(&rarc, "room.kcl").unwrap(),
        b"immutable collision bytes"
    );
    assert_eq!(rarc, original);
    assert_eq!(
        extract_rarc_resource(&yaz0_literals(&rarc), "room.kcl").unwrap(),
        b"immutable collision bytes"
    );
    assert!(extract_rarc_resource(&rarc, "room.plc").is_err());
}

#[test]
fn indexes_full_rarc_paths_and_rejects_directory_cycles() {
    let rarc = indexed_rarc_with("room.kcl", b"immutable collision bytes");
    let parsed = RarcArchive::parse(&rarc).unwrap();
    assert_eq!(parsed.resources().len(), 1);
    assert_eq!(parsed.resources()[0].path, "root/room.kcl");
    assert_eq!(
        parsed.resources()[0].sha256,
        sha256(b"immutable collision bytes")
    );
    assert_eq!(
        parsed.resource("root/room.kcl").unwrap(),
        b"immutable collision bytes"
    );
    assert_eq!(
        parsed.unique_basename("room.kcl").unwrap(),
        b"immutable collision bytes"
    );

    let mut cycle = rarc;
    let file_table = 0x50;
    put_u16(&mut cycle, file_table + 4, 0x0200);
    put_u32(&mut cycle, file_table + 8, 0);
    assert!(
        RarcArchive::parse(&cycle)
            .unwrap_err()
            .to_string()
            .contains("cycle")
    );
}

#[test]
fn rejects_truncated_yaz0_and_oversized_declared_output() {
    assert!(extract_rarc_resource(b"Yaz0", "room.kcl").is_err());
    let mut oversized = vec![0_u8; 16];
    oversized[0..4].copy_from_slice(b"Yaz0");
    put_u32(
        &mut oversized,
        4,
        (MAX_DECOMPRESSED_ARCHIVE_SIZE as u32) + 1,
    );
    assert!(extract_rarc_resource(&oversized, "room.kcl").is_err());
}
