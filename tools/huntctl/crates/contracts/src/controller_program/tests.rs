use super::*;

const SOURCE: &str = r#"
    # bounded controller example
    duskcontrol 1
    frames 120
    bezier replace from 0 for 120 p0 0 127 p1 0 127 p2 127 0 p3 127 0
    seek point add from 0 for 120 target 1 2 3 offset 0 0 0 magnitude 20 stop 5
    seek actor add from 10 for 40 actor 123 offset 1 0 2 magnitude 30 stop 10
    buttons from 5 for 1 B START // overlays are OR-composed
"#;

#[test]
fn parses_encodes_and_decodes_all_layer_kinds() {
    let program = parse(SOURCE).unwrap();
    assert_eq!(program.duration_frames, 120);
    assert_eq!(program.layers.len(), 4);
    let bytes = program.encode().unwrap();
    assert_eq!(bytes.len(), HEADER_SIZE + 4 * RECORD_SIZE);
    assert_eq!(&bytes[..8], MAGIC);
    assert_eq!(get_u16(&bytes, 8), 1);
    assert_eq!(get_u16(&bytes, 10), VERSION_MINOR);
    assert_eq!(get_u16(&bytes, 12), 32);
    assert_eq!(get_u16(&bytes, 14), 64);
    assert_eq!(get_u32(&bytes, 16), 120);
    assert_eq!(get_u16(&bytes, 20), 4);
    assert_eq!(get_u32(&bytes, 24), 256);
    assert_eq!(bytes[HEADER_SIZE], KIND_CUBIC_BEZIER);
    assert_eq!(get_i16(&bytes, HEADER_SIZE + 14), 127);
    assert_eq!(bytes[HEADER_SIZE + 2 * RECORD_SIZE], KIND_SEEK_ACTOR);
    assert_eq!(get_i16(&bytes, HEADER_SIZE + 2 * RECORD_SIZE + 12), 123);
    assert_eq!(bytes[HEADER_SIZE + 3 * RECORD_SIZE + 1], BLEND_OR);
    assert_eq!(get_u16(&bytes, HEADER_SIZE + 3 * RECORD_SIZE + 12), 0x1200);
    assert_eq!(ControllerProgram::decode(&bytes).unwrap(), program);
}

#[test]
fn exact_actor_selectors_round_trip_in_version_1_1() {
    let source = r#"duskcontrol 1
frames 8
seek actor replace from 0 for 4 actor 123 process 42 offset 0 0 0 magnitude 127 stop 0
seek actor replace from 4 for 4 actor -77 set 65535 room -3 stage F_SP103 offset 1 2 3 magnitude 80 stop 5
"#;
    let program = parse(source).unwrap();
    assert_eq!(
        program.layers[0].operation,
        Operation::SeekActor {
            blend: StickBlend::Replace,
            actor_name: 123,
            selector: ActorSelector::Process { process_id: 42 },
            offset: [0.0, 0.0, 0.0],
            stop_radius: 0.0,
            magnitude: 127,
        }
    );
    assert_eq!(
        program.layers[1].operation,
        Operation::SeekActor {
            blend: StickBlend::Replace,
            actor_name: -77,
            selector: ActorSelector::Placed {
                set_id: u16::MAX,
                room: -3,
                stage_name: "F_SP103".to_owned(),
            },
            offset: [1.0, 2.0, 3.0],
            stop_radius: 5.0,
            magnitude: 80,
        }
    );

    let bytes = program.encode().unwrap();
    let first = HEADER_SIZE;
    let second = HEADER_SIZE + RECORD_SIZE;
    assert_eq!(bytes[first + 14], 1);
    assert_eq!(get_u32(&bytes, first + 33), 42);
    assert_eq!(bytes[second + 14], 2);
    assert_eq!(bytes[second + 15] as i8, -3);
    assert_eq!(get_u16(&bytes, second + 37), u16::MAX);
    assert_eq!(&bytes[second + 39..second + 46], b"F_SP103");
    assert_eq!(bytes[second + 46], 0);
    assert_eq!(ControllerProgram::decode(&bytes).unwrap(), program);
}

#[test]
fn framed_coordinate_plane_path_and_opening_targets_round_trip_in_version_1_2() {
    let source = r#"duskcontrol 1
frames 6
seek coordinate replace from 0 for 1 frame world target 10 0 20 offset 1 0 2 magnitude 127 stop 3
seek coordinate replace from 1 for 1 frame player target 0 0 100 offset 0 0 0 magnitude 90 stop 2
seek coordinate replace from 2 for 1 frame camera target 10 0 0 offset 0 0 0 magnitude 80 stop 1
seek plane replace from 3 for 1 frame world point 0 0 50 normal 0 0 2 magnitude 70 stop 4
seek path-point replace from 4 for 1 path 42 point 7 target 1 2 3 offset 4 5 6 magnitude 60 stop 5
seek opening replace from 5 for 1 opening 99 target 7 8 9 offset 1 0 -1 magnitude 50 stop 6
"#;
    let program = parse(source).unwrap();
    assert_eq!(program.layers.len(), 6);
    assert!(matches!(
        program.layers[1].operation,
        Operation::SeekCoordinate {
            frame: CoordinateFrame::Player,
            ..
        }
    ));
    assert!(matches!(
        program.layers[2].operation,
        Operation::SeekCoordinate {
            frame: CoordinateFrame::Camera,
            ..
        }
    ));
    assert!(matches!(
        &program.layers[4].operation,
        Operation::SeekResolved {
            target: ResolvedTarget::PathPoint {
                path_id: 42,
                point_index: 7,
                ..
            },
            ..
        }
    ));
    assert!(matches!(
        &program.layers[5].operation,
        Operation::SeekResolved {
            target: ResolvedTarget::Opening { opening_id: 99, .. },
            ..
        }
    ));
    let bytes = program.encode_for_version(2).unwrap();
    assert_eq!(get_u16(&bytes, 10), 2);
    assert_eq!(bytes[HEADER_SIZE], KIND_SEEK_COORDINATE);
    assert_eq!(bytes[HEADER_SIZE + 3 * RECORD_SIZE], KIND_SEEK_PLANE);
    assert_eq!(bytes[HEADER_SIZE + 4 * RECORD_SIZE], KIND_SEEK_RESOLVED);
    assert_eq!(ControllerProgram::decode(&bytes).unwrap(), program);
    assert!(
        program
            .encode_for_version(1)
            .unwrap_err()
            .to_string()
            .contains("version 1.2")
    );
}

#[test]
fn turn_brake_neutral_heading_and_distance_round_trip_in_version_1_3() {
    let source = r#"duskcontrol 1
frames 6
neutral replace from 0 for 1
turn replace from 1 for 1 direction left magnitude 40
brake replace from 2 for 1 stop-speed 0.5 magnitude 50
align replace from 3 for 1 frame world heading 1.5 tolerance 0.1 magnitude 60
maintain heading replace from 4 for 1 frame camera heading -0.25 magnitude 70
maintain distance replace from 5 for 1 frame world target 10 0 20 distance 5 tolerance 1 magnitude 80
"#;
    let program = parse(source).unwrap();
    assert_eq!(program.layers.len(), 6);
    assert!(matches!(program.layers[0].operation, Operation::Neutral));
    assert!(matches!(
        program.layers[1].operation,
        Operation::Turn {
            direction: TurnDirection::Left,
            ..
        }
    ));
    assert!(matches!(
        program.layers[2].operation,
        Operation::Brake { .. }
    ));
    assert!(matches!(
        program.layers[3].operation,
        Operation::Align { .. }
    ));
    assert!(matches!(
        program.layers[4].operation,
        Operation::MaintainHeading { .. }
    ));
    assert!(matches!(
        program.layers[5].operation,
        Operation::MaintainDistance { .. }
    ));
    let bytes = program.encode().unwrap();
    assert_eq!(get_u16(&bytes, 10), VERSION_MINOR);
    assert_eq!(bytes[HEADER_SIZE], KIND_NEUTRAL);
    assert_eq!(bytes[HEADER_SIZE + RECORD_SIZE], KIND_TURN);
    assert_eq!(bytes[HEADER_SIZE + 2 * RECORD_SIZE], KIND_BRAKE);
    assert_eq!(bytes[HEADER_SIZE + 3 * RECORD_SIZE], KIND_HEADING);
    assert_eq!(bytes[HEADER_SIZE + 4 * RECORD_SIZE], KIND_HEADING);
    assert_eq!(bytes[HEADER_SIZE + 5 * RECORD_SIZE], KIND_MAINTAIN_DISTANCE);
    assert_eq!(ControllerProgram::decode(&bytes).unwrap(), program);
    assert!(program.encode_for_version(2).is_err());
}

#[test]
fn motion_control_syntax_values_and_binary_reservations_are_strict() {
    for source in [
        "duskcontrol 1\nframes 1\nneutral add from 0 for 1\n",
        "duskcontrol 1\nframes 1\nturn replace from 0 for 1 direction up magnitude 1\n",
        "duskcontrol 1\nframes 1\nbrake replace from 0 for 1 stop-speed -1 magnitude 1\n",
        "duskcontrol 1\nframes 1\nalign replace from 0 for 1 frame world heading 4 tolerance 0 magnitude 1\n",
        "duskcontrol 1\nframes 1\nalign replace from 0 for 1 frame world heading 0 tolerance 4 magnitude 1\n",
        "duskcontrol 1\nframes 1\nmaintain distance replace from 0 for 1 frame world target 0 0 1 distance 1 tolerance 2 magnitude 1\n",
    ] {
        assert!(parse(source).is_err(), "unexpectedly accepted {source:?}");
    }

    let program = parse(
        "duskcontrol 1\nframes 1\nmaintain heading replace from 0 for 1 frame world heading 0 magnitude 1\n",
    )
    .unwrap();
    let mut bytes = program.encode().unwrap();
    bytes[HEADER_SIZE + 15] = 1;
    assert!(ControllerProgram::decode(&bytes).is_err());
    let mut bytes = program.encode().unwrap();
    put_f32(&mut bytes, HEADER_SIZE + 20, -0.0);
    assert!(ControllerProgram::decode(&bytes).is_err());
}

#[test]
fn new_target_syntax_and_binary_reservations_are_strict() {
    for source in [
        "duskcontrol 1\nframes 1\nseek coordinate replace from 0 for 1 frame local target 0 0 1 offset 0 0 0 magnitude 1 stop 0\n",
        "duskcontrol 1\nframes 1\nseek plane replace from 0 for 1 frame world point 0 0 1 normal 0 1 0 magnitude 1 stop 0\n",
        "duskcontrol 1\nframes 1\nseek path-point replace from 0 for 1 path 0 point 1 target 0 0 1 offset 0 0 0 magnitude 1 stop 0\n",
        "duskcontrol 1\nframes 1\nseek opening replace from 0 for 1 opening 0 target 0 0 1 offset 0 0 0 magnitude 1 stop 0\n",
    ] {
        assert!(parse(source).and_then(|program| program.encode()).is_err());
    }

    let program = parse(
        "duskcontrol 1\nframes 1\nseek opening replace from 0 for 1 opening 9 target 0 0 1 offset 0 0 0 magnitude 1 stop 0\n",
    )
    .unwrap();
    let mut bytes = program.encode().unwrap();
    bytes[HEADER_SIZE + 13] = 1;
    assert!(ControllerProgram::decode(&bytes).is_err());
    let mut bytes = program.encode().unwrap();
    put_u32(&mut bytes, HEADER_SIZE + 24, 1);
    assert!(ControllerProgram::decode(&bytes).is_err());
}

#[test]
fn version_1_0_nearest_actor_programs_remain_decodable_and_strict() {
    let program = parse(SOURCE).unwrap();
    let legacy = program.encode_for_version(0).unwrap();
    assert_eq!(get_u16(&legacy, 10), 0);
    assert_eq!(ControllerProgram::decode(&legacy).unwrap(), program);

    let actor = HEADER_SIZE + 2 * RECORD_SIZE;
    let mut selector_in_legacy = legacy.clone();
    selector_in_legacy[actor + 14] = 1;
    assert!(
        ControllerProgram::decode(&selector_in_legacy)
            .unwrap_err()
            .to_string()
            .contains("reserved")
    );

    let exact = parse("duskcontrol 1\nframes 1\nseek actor add from 0 for 1 actor 1 process 2 offset 0 0 0 magnitude 1 stop 0\n").unwrap();
    assert!(
        exact
            .encode_for_version(0)
            .unwrap_err()
            .to_string()
            .contains("version 1.1")
    );
}

#[test]
fn actor_selector_syntax_and_values_are_strict() {
    let prefix = "duskcontrol 1\nframes 1\nseek actor add from 0 for 1 actor 1 ";
    let suffix = " offset 0 0 0 magnitude 1 stop 0\n";
    for (selector, expected) in [
        ("process 0", "process ID"),
        ("process 4294967295", "process ID"),
        ("set 1", "room field"),
        ("room 1", "set field"),
        ("set 1 room 0", "stage field"),
        ("stage F_SP103", "requires a placed"),
        ("set 1 room 128 stage F_SP103", "invalid room"),
        ("set 1 room -129 stage F_SP103", "invalid room"),
        ("set 1 room 0 stage TOO_LONG9", "at most 8"),
        ("set 1 room 0 stage F_SP10é", "ASCII"),
        ("process 5 set 1 room 0", "mutually exclusive"),
        ("process 5 stage F_SP103", "mutually exclusive"),
        ("process 5 process 6", "duplicate process"),
        ("set 1 room 0 stage F_SP103 room 1", "duplicate room"),
        ("set 1 room 0 stage F_SP103 stage D_MN01", "duplicate stage"),
    ] {
        let source = format!("{prefix}{selector}{suffix}");
        assert!(
            parse(&source).unwrap_err().to_string().contains(expected),
            "selector {selector:?} did not report {expected:?}"
        );
    }
}

#[test]
fn actor_selector_binary_reservations_are_canonical() {
    let process = parse("duskcontrol 1\nframes 1\nseek actor add from 0 for 1 actor 1 process 2 offset 0 0 0 magnitude 1 stop 0\n").unwrap();
    let mut process_room = process.encode().unwrap();
    process_room[HEADER_SIZE + 15] = 1;
    assert!(ControllerProgram::decode(&process_room).is_err());
    let mut process_set = process.encode().unwrap();
    process_set[HEADER_SIZE + 37] = 1;
    assert!(ControllerProgram::decode(&process_set).is_err());

    let placed = parse("duskcontrol 1\nframes 1\nseek actor add from 0 for 1 actor 1 set 2 room 0 stage F_SP103 offset 0 0 0 magnitude 1 stop 0\n").unwrap();
    let mut placed_process = placed.encode().unwrap();
    placed_process[HEADER_SIZE + 33] = 1;
    assert!(ControllerProgram::decode(&placed_process).is_err());
    let mut placed_tail = placed.encode().unwrap();
    placed_tail[HEADER_SIZE + 47] = 1;
    assert!(ControllerProgram::decode(&placed_tail).is_err());
    let mut placed_gap = placed.encode().unwrap();
    placed_gap[HEADER_SIZE + 42] = 0;
    assert!(ControllerProgram::decode(&placed_gap).is_err());
    let mut placed_empty = placed.encode().unwrap();
    placed_empty[HEADER_SIZE + 39..HEADER_SIZE + 47].fill(0);
    assert!(ControllerProgram::decode(&placed_empty).is_err());
    let mut placed_non_ascii = placed.encode().unwrap();
    placed_non_ascii[HEADER_SIZE + 39] = 0x80;
    assert!(ControllerProgram::decode(&placed_non_ascii).is_err());
    let mut unknown_mode = placed.encode().unwrap();
    unknown_mode[HEADER_SIZE + 14] = 3;
    assert!(
        ControllerProgram::decode(&unknown_mode)
            .unwrap_err()
            .to_string()
            .contains("invalid actor selector mode")
    );
}

#[test]
fn field_order_is_flexible_but_duplicates_and_unknowns_fail() {
    let reordered = "duskcontrol 1\nframes 4\nseek point add from 0 for 4 stop 0 magnitude 127 offset 0 0 0 target 1 2 3\n";
    assert!(parse(reordered).is_ok());
    let duplicate = "duskcontrol 1\nframes 4\nseek point add from 0 for 4 stop 0 stop 1 magnitude 1 offset 0 0 0 target 1 2 3\n";
    assert!(
        parse(duplicate)
            .unwrap_err()
            .to_string()
            .contains("duplicate stop")
    );
    let unknown = "duskcontrol 1\nframes 4\nseek point add from 0 for 4 target 1 2 3 offset 0 0 0 magnitude 1 stop 0 wat 4\n";
    assert!(parse(unknown).unwrap_err().to_string().contains("unknown"));
}

#[test]
fn rejects_overlapping_replace_stick_layers_but_allows_additive_layers() {
    let overlapping = "duskcontrol 1\nframes 20\nbezier replace from 0 for 11 p0 0 0 p1 0 0 p2 0 0 p3 0 0\nseek point replace from 10 for 10 target 0 0 0 offset 0 0 0 magnitude 1 stop 0\n";
    assert!(
        parse(overlapping)
            .unwrap_err()
            .to_string()
            .contains("overlap")
    );
    let adjacent = overlapping.replace("from 10 for 10", "from 11 for 9");
    assert!(parse(&adjacent).is_ok());
}

#[test]
fn camera_and_safety_clamp_round_trip_with_independent_writer_surfaces() {
    let source = r#"duskcontrol 1
frames 4
bezier replace from 0 for 4 p0 120 -120 p1 120 -120 p2 120 -120 p3 120 -120
camera replace from 0 for 4 x 100 y -100
camera add from 0 for 4 x 40 y 40
buttons from 0 for 4 B
clamp from 0 for 4 main 90 substick 80
"#;
    let program = parse(source).unwrap();
    assert_eq!(program.layers.len(), 5);
    assert!(matches!(
        program.layers[1].operation,
        Operation::Camera {
            blend: StickBlend::Replace,
            x: 100,
            y: -100
        }
    ));
    assert!(matches!(
        program.layers[4].operation,
        Operation::SafetyClamp {
            main_limit: 90,
            substick_limit: 80
        }
    ));
    let bytes = program.encode().unwrap();
    assert_eq!(get_u16(&bytes, 10), 4);
    assert_eq!(bytes[HEADER_SIZE + RECORD_SIZE], KIND_CAMERA);
    assert_eq!(bytes[HEADER_SIZE + 4 * RECORD_SIZE], KIND_SAFETY_CLAMP);
    assert_eq!(ControllerProgram::decode(&bytes).unwrap(), program);
    assert!(program.encode_for_version(3).is_err());

    let overlapping_camera = source.replace(
        "camera add from 0 for 4 x 40 y 40",
        "camera replace from 0 for 4 x 40 y 40",
    );
    assert!(parse(&overlapping_camera).is_err());
    let overlapping_clamp = source.replace(
        "clamp from 0 for 4 main 90 substick 80",
        "clamp from 0 for 4 main 90 substick 80\nclamp from 1 for 1 main 1 substick 1",
    );
    assert!(parse(&overlapping_clamp).is_err());
}

#[test]
fn validates_ranges_floats_magnitudes_and_buttons() {
    for (source, expected) in [
        (
            "duskcontrol 1\nframes 4\nbuttons from 4 for 1 A\n",
            "exceeds",
        ),
        (
            "duskcontrol 1\nframes 4\nbuttons from 0 for 0 A\n",
            "zero duration",
        ),
        (
            "duskcontrol 1\nframes 4\nseek point add from 0 for 4 target NaN 0 0 offset 0 0 0 magnitude 1 stop 0\n",
            "non-finite",
        ),
        (
            "duskcontrol 1\nframes 4\nseek actor add from 0 for 4 actor 1 offset 0 0 0 magnitude 0 stop 0\n",
            "magnitude",
        ),
        (
            "duskcontrol 1\nframes 4\nseek actor add from 0 for 4 actor 1 offset 0 0 0 magnitude 1 stop -1\n",
            "nonnegative",
        ),
        (
            "duskcontrol 1\nframes 4\nbuttons from 0 for 1 NOPE\n",
            "unknown button",
        ),
        (
            "duskcontrol 1\nframes 4\nbuttons from 0 for 1 START MENU\n",
            "duplicate button",
        ),
    ] {
        assert!(parse(source).unwrap_err().to_string().contains(expected));
    }
}

#[test]
fn rejects_noncanonical_binary_fields_and_trailing_data() {
    let canonical = parse(SOURCE).unwrap().encode().unwrap();
    for (offset, expected) in [
        (22, "reserved header"),
        (28, "reserved header"),
        (HEADER_SIZE + 2, "port"),
        (HEADER_SIZE + 3, "flags"),
        (HEADER_SIZE + 28, "reserved payload"),
    ] {
        let mut corrupt = canonical.clone();
        corrupt[offset] = 1;
        assert!(
            ControllerProgram::decode(&corrupt)
                .unwrap_err()
                .to_string()
                .contains(expected)
        );
    }
    let mut trailing = canonical;
    trailing.push(0);
    assert!(ControllerProgram::decode(&trailing).is_err());
}

#[test]
fn rejects_wrong_blend_and_nonfinite_binary_float() {
    let program = parse(SOURCE).unwrap();
    let mut wrong_button_blend = program.encode().unwrap();
    wrong_button_blend[HEADER_SIZE + 3 * RECORD_SIZE + 1] = BLEND_ADD;
    assert!(
        ControllerProgram::decode(&wrong_button_blend)
            .unwrap_err()
            .to_string()
            .contains("OR blend")
    );

    let mut nan = program.encode().unwrap();
    nan[HEADER_SIZE + RECORD_SIZE + 12..HEADER_SIZE + RECORD_SIZE + 16]
        .copy_from_slice(&f32::NAN.to_le_bytes());
    assert!(
        ControllerProgram::decode(&nan)
            .unwrap_err()
            .to_string()
            .contains("non-finite")
    );
}

#[test]
fn every_truncation_and_unknown_kind_is_rejected() {
    let canonical = parse(SOURCE).unwrap().encode().unwrap();
    for length in 0..canonical.len() {
        assert!(
            ControllerProgram::decode(&canonical[..length]).is_err(),
            "accepted truncation at {length} bytes"
        );
    }
    let mut unknown_kind = canonical;
    unknown_kind[HEADER_SIZE] = 99;
    assert!(
        ControllerProgram::decode(&unknown_kind)
            .unwrap_err()
            .to_string()
            .contains("unknown kind")
    );
}

#[test]
fn duration_and_layer_limits_are_inclusive_and_enforced() {
    let boundary = format!("duskcontrol 1\nframes {MAX_DURATION_FRAMES}\n");
    assert!(parse(&boundary).is_ok());
    let too_long = format!("duskcontrol 1\nframes {}\n", MAX_DURATION_FRAMES + 1);
    assert!(
        parse(&too_long)
            .unwrap_err()
            .to_string()
            .contains("duration")
    );

    let mut maximum_layers = String::from("duskcontrol 1\nframes 1\n");
    for _ in 0..MAX_LAYERS {
        maximum_layers.push_str("buttons from 0 for 1 A\n");
    }
    assert_eq!(parse(&maximum_layers).unwrap().layers.len(), MAX_LAYERS);
    maximum_layers.push_str("buttons from 0 for 1 B\n");
    assert!(
        parse(&maximum_layers)
            .unwrap_err()
            .to_string()
            .contains("at most")
    );
}

#[test]
fn requires_unique_header_and_frames_declarations() {
    assert!(parse("").unwrap_err().to_string().contains("missing"));
    assert!(
        parse("frames 1\n")
            .unwrap_err()
            .to_string()
            .contains("first")
    );
    assert!(
        parse("duskcontrol 1\n")
            .unwrap_err()
            .to_string()
            .contains("frames")
    );
    assert!(
        parse("duskcontrol 1\nframes 1\nframes 1\n")
            .unwrap_err()
            .to_string()
            .contains("duplicate frames")
    );
}
