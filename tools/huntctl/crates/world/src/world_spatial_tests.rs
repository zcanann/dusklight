use super::*;
use crate::world_geometry::{CollisionCode, KclInventoryPrism, KclSourceIndices};
use crate::world_inventory::WORLD_INVENTORY_SCHEMA;
use crate::world_inventory::{CollisionInventoryRecord, SourceKind, SourceScope, WorldSource};

fn authored(id: &str, index: u16) -> KclAuthoredPrism {
    KclAuthoredPrism {
        stable_id: id.into(),
        prism_index: index,
        height: 1.0,
        source_indices: KclSourceIndices {
            position: 0,
            face_normal: 0,
            edge_normal_1: 0,
            edge_normal_2: 0,
            edge_normal_3: 0,
        },
        attribute: index,
        code: CollisionCode {
            raw: [0; 5],
            exit_id: 0x3f,
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
    }
}

fn triangle_at(x: f32) -> (CollisionPlane, [Vec3; 3]) {
    let anchor = Vec3 { x, y: 0.0, z: 0.0 };
    (
        CollisionPlane {
            anchor,
            normal: Vec3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
            d: 0.0,
        },
        [
            anchor,
            Vec3 { x, y: 0.0, z: 1.0 },
            Vec3 {
                x: x + 1.0,
                y: 0.0,
                z: 0.0,
            },
        ],
    )
}

fn inventory() -> WorldInventory {
    let mut collisions = Vec::new();
    for (index, x) in [0.0_f32, 10.0, 20.0].into_iter().enumerate() {
        let (plane, triangle) = triangle_at(x);
        collisions.push(CollisionInventoryRecord {
            room: if index == 2 { 1 } else { 0 },
            prism: KclInventoryPrism {
                authored: authored(&format!("surface/{index}"), index as u16 + 1),
                reconstruction: KclReconstruction::Reconstructed { plane, triangle },
            },
        });
    }
    collisions.push(CollisionInventoryRecord {
        room: 0,
        prism: KclInventoryPrism {
            authored: authored("surface/degenerate", 4),
            reconstruction: KclReconstruction::Degenerate {
                reason: "synthetic".into(),
            },
        },
    });
    let trigger = CollisionLoadTrigger {
        stable_id: "trigger/0".into(),
        room: 0,
        collision_id: "surface/1".into(),
        collision_exit_id: 0,
        scls_id: "exit/0".into(),
        destination_stage: "NEXT".into(),
        destination_room: 0,
        destination_layer: -1,
        destination_point: 0,
        inferred_semantics: true,
    };
    WorldInventory {
        schema: WORLD_INVENTORY_SCHEMA.into(),
        stage: "TEST".into(),
        sources: vec![WorldSource {
            scope: SourceScope {
                kind: SourceKind::Room,
                room: Some(0),
            },
            archive_sha256: Digest([0; 32]),
            stage_data_path: "room.dzr".into(),
            stage_data_sha256: Digest([1; 32]),
            kcl_path: Some("room.kcl".into()),
            kcl_sha256: Some(Digest([2; 32])),
            plc_path: Some("room.plc".into()),
            plc_sha256: Some(Digest([3; 32])),
            addressable_prisms: 4,
        }],
        chunks: Vec::new(),
        placements: Vec::new(),
        player_spawns: Vec::new(),
        exits: Vec::new(),
        paths: Vec::new(),
        path_points: Vec::new(),
        collisions,
        load_triggers: vec![trigger],
    }
}

fn filter(room: i8) -> WorldSurfaceFilter {
    WorldSurfaceFilter {
        room,
        load_triggers_only: false,
        trigger_stable_id: None,
        destination_stage: None,
        destination_room: None,
        destination_point: None,
    }
}

#[test]
fn point_queries_are_ranked_bounded_filtered_and_explicit_about_degeneracy() {
    let inventory = inventory();
    let index = WorldSpatialIndex::build(&inventory).unwrap();
    let second = WorldSpatialIndex::build(&inventory).unwrap();
    assert_eq!(
        index.artifact().canonical_bytes().unwrap(),
        second.artifact().canonical_bytes().unwrap()
    );
    assert_eq!(
        index.artifact_digest().unwrap(),
        second.artifact_digest().unwrap()
    );
    assert_eq!(index.artifact().rooms.len(), 2);
    assert_eq!(index.artifact().excluded.len(), 1);
    assert_eq!(index.artifact().rooms[0].root, Some(0));
    assert!(index.artifact().rooms[0].primitive_ids.is_sorted());
    assert_eq!(
        index
            .artifact()
            .rooms
            .iter()
            .map(|room| room.primitive_ids.len())
            .sum::<usize>()
            + index.artifact().excluded.len(),
        inventory.collisions.len()
    );
    let report = index
        .point_query(WorldPointQueryRequest {
            point: Vec3 {
                x: 0.25,
                y: 2.0,
                z: 0.25,
            },
            max_distance: None,
            limit: 1,
            filter: filter(0),
        })
        .unwrap();
    assert_eq!(report.indexed_surface_count, 2);
    assert_eq!(report.excluded_degenerate_count, 1);
    assert_eq!(report.within_distance_count, 2);
    assert_eq!(report.returned_count, 1);
    assert!(report.truncated);
    assert_eq!(report.results[0].surface.authored.stable_id, "surface/0");
    assert!((report.results[0].point_query.distance - 2.0).abs() < 1e-6);

    let load_only = index
        .point_query(WorldPointQueryRequest {
            point: Vec3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            max_distance: Some(11.0),
            limit: 4,
            filter: WorldSurfaceFilter {
                room: 0,
                load_triggers_only: true,
                trigger_stable_id: None,
                destination_stage: None,
                destination_room: None,
                destination_point: None,
            },
        })
        .unwrap();
    assert_eq!(load_only.eligible_surface_count, 1);
    assert_eq!(load_only.results[0].surface.authored.stable_id, "surface/1");
    assert!(load_only.results[0].surface.load_trigger.is_some());

    let exact_destination = index
        .point_query(WorldPointQueryRequest {
            point: Vec3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            max_distance: None,
            limit: 1,
            filter: WorldSurfaceFilter {
                destination_stage: Some("NEXT".into()),
                ..filter(0)
            },
        })
        .unwrap();
    assert_eq!(exact_destination.eligible_surface_count, 1);
    assert_eq!(
        exact_destination.results[0].surface.authored.stable_id,
        "surface/1"
    );
}

#[test]
fn aabb_and_double_sided_ray_queries_use_exact_bounded_results() {
    let inventory = inventory();
    let index = WorldSpatialIndex::build(&inventory).unwrap();
    let area = index
        .aabb_query(WorldAabbQueryRequest {
            bounds: Aabb3::new(
                Vec3 {
                    x: -1.0,
                    y: -1.0,
                    z: -1.0,
                },
                Vec3 {
                    x: 11.0,
                    y: 1.0,
                    z: 2.0,
                },
            )
            .unwrap(),
            limit: 8,
            filter: filter(0),
        })
        .unwrap();
    assert_eq!(area.overlapping_aabb_count, 2);

    let ray = index
        .ray_query(WorldRayQueryRequest {
            origin: Vec3 {
                x: 0.25,
                y: 3.0,
                z: 0.25,
            },
            direction: Vec3 {
                x: 0.0,
                y: -2.0,
                z: 0.0,
            },
            max_distance: 10.0,
            limit: 4,
            filter: filter(0),
        })
        .unwrap();
    assert_eq!(ray.hit_count, 1);
    assert_eq!(ray.results[0].surface.authored.stable_id, "surface/0");
    assert!((ray.results[0].distance - 3.0).abs() < 1e-6);
    assert!(ray.results[0].front_facing);
    assert!((ray.results[0].barycentric.iter().sum::<f32>() - 1.0).abs() < 1e-6);
}

#[test]
fn query_validation_rejects_ambiguous_or_unbounded_requests() {
    assert!(
        Aabb3::new(
            Vec3 {
                x: 1.0,
                y: 0.0,
                z: 0.0
            },
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: 0.0
            }
        )
        .is_err()
    );
    let inventory = inventory();
    let index = WorldSpatialIndex::build(&inventory).unwrap();
    assert!(
        index
            .point_query(WorldPointQueryRequest {
                point: Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0
                },
                max_distance: None,
                limit: 0,
                filter: filter(0),
            })
            .is_err()
    );
    assert!(
        index
            .ray_query(WorldRayQueryRequest {
                origin: Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0
                },
                direction: Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0
                },
                max_distance: 1.0,
                limit: 1,
                filter: filter(0),
            })
            .is_err()
    );
}

#[test]
fn bvh_nearest_matches_brute_force_and_ignores_source_enumeration_order() {
    let mut fixture = inventory();
    fixture.collisions.clear();
    fixture.load_triggers.clear();
    for index in 0..96_u16 {
        let x = f32::from(index % 12) * 37.0 - 180.0;
        let z = f32::from(index / 12) * 41.0 - 140.0;
        let y = f32::from(index % 7) * 3.0;
        let anchor = Vec3 { x, y, z };
        fixture.collisions.push(CollisionInventoryRecord {
            room: 0,
            prism: KclInventoryPrism {
                authored: authored(&format!("surface/{index:03}"), index + 1),
                reconstruction: KclReconstruction::Reconstructed {
                    plane: CollisionPlane {
                        anchor,
                        normal: Vec3 {
                            x: 0.0,
                            y: 1.0,
                            z: 0.0,
                        },
                        d: -y,
                    },
                    triangle: [
                        anchor,
                        Vec3 { x: x + 21.0, y, z },
                        Vec3 { x, y, z: z + 19.0 },
                    ],
                },
            },
        });
    }
    fixture.sources[0].addressable_prisms = fixture.collisions.len();
    let index = WorldSpatialIndex::build(&fixture).unwrap();

    let mut seed = 0x1234_5678_u32;
    for sample in 0..200 {
        let mut next = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / (u32::MAX >> 8) as f32
        };
        let point = Vec3 {
            x: next() * 500.0 - 250.0,
            y: next() * 120.0 - 30.0,
            z: next() * 450.0 - 200.0,
        };
        let limit = [1, 3, 8][sample % 3];
        let report = index
            .point_query(WorldPointQueryRequest {
                point,
                max_distance: None,
                limit,
                filter: filter(0),
            })
            .unwrap();

        let mut brute = fixture
            .collisions
            .iter()
            .map(|collision| {
                let KclReconstruction::Reconstructed { plane, triangle } =
                    collision.prism.reconstruction
                else {
                    unreachable!()
                };
                (
                    collision.prism.authored.stable_id.as_str(),
                    query_triangle_point(plane, triangle, point)
                        .unwrap()
                        .distance,
                )
            })
            .collect::<Vec<_>>();
        brute.sort_by(|left, right| left.1.total_cmp(&right.1).then_with(|| left.0.cmp(right.0)));
        assert_eq!(report.results.len(), limit);
        for (result, expected) in report.results.iter().zip(&brute[..limit]) {
            assert_eq!(result.surface.authored.stable_id, expected.0);
            assert_eq!(result.point_query.distance.to_bits(), expected.1.to_bits());
        }
    }

    let mut reversed = fixture.clone();
    reversed.collisions.reverse();
    let reversed_index = WorldSpatialIndex::build(&reversed).unwrap();
    assert_eq!(index.artifact().rooms, reversed_index.artifact().rooms);
    assert_eq!(
        index.artifact().excluded,
        reversed_index.artifact().excluded
    );
}

#[test]
fn real_f_sp103_spatial_goldens_match_when_disc_is_present() {
    let stage_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../..")
        .join("orig/GZ2E01/files/res/Stage/F_SP103");
    if !stage_dir.is_dir() {
        eprintln!("skipping F_SP103 spatial golden: original disc data is absent");
        return;
    }
    let inventory = WorldInventory::build(&stage_dir, "F_SP103").unwrap();
    let index = WorldSpatialIndex::build(&inventory).unwrap();
    assert_eq!(
        index.artifact_digest().unwrap().to_string(),
        "dda6381f80f735821eea4a199510568281980978c902d963b9f8684db7dc4d1a"
    );
    let room0 = index
        .artifact()
        .rooms
        .iter()
        .find(|room| room.room == 0)
        .unwrap();
    let room1 = index
        .artifact()
        .rooms
        .iter()
        .find(|room| room.room == 1)
        .unwrap();
    assert_eq!(room0.primitive_ids.len(), 8_566);
    assert_eq!(room1.primitive_ids.len(), 2_224);
    assert_eq!(index.artifact().excluded.len(), 4);

    let load_trigger_accounting = index
        .point_query(WorldPointQueryRequest {
            point: Vec3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            max_distance: None,
            limit: 1,
            filter: WorldSurfaceFilter {
                load_triggers_only: true,
                ..filter(1)
            },
        })
        .unwrap();
    assert_eq!(load_trigger_accounting.excluded_matching_filter_count, 1);

    // This point is deliberately closer to an ordinary surface than to
    // the desired exit. Destination filtering must happen before ranking.
    let point = Vec3 {
        x: -1_965.393_3,
        y: 821.341_8,
        z: -4364.942,
    };
    let nearest_any = index
        .point_query(WorldPointQueryRequest {
            point,
            max_distance: None,
            limit: 1,
            filter: filter(1),
        })
        .unwrap();
    assert_eq!(nearest_any.results[0].surface.authored.prism_index, 2187);
    assert!(nearest_any.triangle_tests < nearest_any.eligible_surface_count);

    let nearest_destination = index
        .point_query(WorldPointQueryRequest {
            point,
            max_distance: None,
            limit: 1,
            filter: WorldSurfaceFilter {
                destination_stage: Some("F_SP104".into()),
                ..filter(1)
            },
        })
        .unwrap();
    assert_eq!(
        nearest_destination.results[0].surface.authored.prism_index,
        2217
    );
    assert!((nearest_destination.results[0].point_query.distance - 100.0).abs() < 1.0e-3);

    let live_point = index
        .point_query(WorldPointQueryRequest {
            point: Vec3 {
                x: -2_037.332_4,
                y: 729.72,
                z: -4264.551,
            },
            max_distance: Some(0.001),
            limit: 4,
            filter: WorldSurfaceFilter {
                destination_stage: Some("F_SP104".into()),
                ..filter(1)
            },
        })
        .unwrap();
    assert_eq!(live_point.results[0].surface.authored.prism_index, 2217);
    assert!(live_point.results[0].point_query.distance < 1.0e-3);

    let ray = index
        .ray_query(WorldRayQueryRequest {
            origin: Vec3 {
                x: -1_970.221_8,
                y: 771.584_2,
                z: -4364.008,
            },
            direction: Vec3 {
                x: -0.096_570_1,
                y: -0.995_150_86,
                z: 0.018_679_99,
            },
            max_distance: 60.0,
            limit: 4,
            filter: WorldSurfaceFilter {
                destination_stage: Some("F_SP104".into()),
                ..filter(1)
            },
        })
        .unwrap();
    assert_eq!(ray.results[0].surface.authored.prism_index, 2217);
    assert!((ray.results[0].distance - 50.0).abs() < 1.0e-3);
    assert!(ray.results[0].front_facing);
}
