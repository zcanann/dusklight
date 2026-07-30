use super::*;
use dusklight_control::controller_program::{ControllerProgram, Layer, StickBlend};
use dusklight_control::game_tactic::{GameTactic, GameTacticPlan};
use dusklight_control::motion_path::{MotionPathPlan, SamplePhase, StickPath, StickPoint};
use dusklight_control::roll_option::{RollOptionPlan, RollSpacing};

fn parameter_f32(description: &TacticAssetDescription, name: &str) -> f32 {
    match description.option.parameters.get(name).unwrap() {
        OptionParameter::F32Bits(bits) => f32::from_bits(*bits),
        parameter => panic!("{name} was not an f32 parameter: {parameter:?}"),
    }
}

#[test]
fn reactive_controller_describes_generic_motion_and_button_factors() {
    let controller = ControllerProgram {
        duration_frames: 12,
        layers: vec![
            Layer {
                start_frame: 0,
                duration_frames: 12,
                operation: Operation::SeekCoordinateSequence {
                    blend: StickBlend::Replace,
                    coordinates_xz: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]],
                    intermediate_stop_radius: 1.0,
                    final_stop_radius: 0.0,
                    magnitude: 127,
                },
            },
            Layer {
                start_frame: 1,
                duration_frames: 1,
                operation: Operation::Buttons { mask: 0x0100 },
            },
            Layer {
                start_frame: 5,
                duration_frames: 1,
                operation: Operation::Buttons { mask: 0x0100 },
            },
        ],
    };

    let description = controller.describe("opaque-controller-id").unwrap();

    assert_eq!(
        description.option.parameters.get("command_button_mask"),
        Some(&OptionParameter::Unsigned(0x0100))
    );
    assert_eq!(
        description
            .option
            .parameters
            .get("command_target_point_count"),
        Some(&OptionParameter::Unsigned(3))
    );
    assert_eq!(
        description.option.parameters.get("command_stick_magnitude"),
        Some(&OptionParameter::Unsigned(127))
    );
    assert_eq!(
        parameter_f32(&description, "command_internal_path_length"),
        20.0
    );
    assert!(
        (parameter_f32(&description, "command_internal_displacement") - 10.0_f32.hypot(10.0)).abs()
            < 1.0e-5
    );
    assert!(
        (parameter_f32(&description, "command_internal_turn_radians")
            - std::f32::consts::FRAC_PI_2)
            .abs()
            < 1.0e-5
    );
    assert_eq!(
        parameter_f32(&description, "command_button_mean_interval_ticks"),
        4.0
    );
}

#[test]
fn static_controller_summary_preserves_setup_heading_and_button_phase() {
    let controller = ControllerProgram::parse(
        "duskcontrol 1\nframes 4\n\
             bezier replace from 0 for 2 p0 -127 0 p1 -127 0 p2 -127 0 p3 -127 0\n\
             buttons from 1 for 1 L\n\
             bezier replace from 2 for 2 p0 0 127 p1 0 127 p2 0 127 p3 0 127\n",
    )
    .unwrap();

    let description = controller.describe("camera-lock-forward").unwrap();

    assert_eq!(description.option.option_type, OptionType::Target);
    assert!(
        (parameter_f32(&description, "command_initial_heading") - std::f32::consts::FRAC_PI_2)
            .abs()
            < 1.0e-6
    );
    assert_eq!(
        description.option.parameters.get("button_pulse_phase_tick"),
        Some(&OptionParameter::Unsigned(1))
    );
    assert_eq!(
        description.option.parameters.get("command_button_mask"),
        Some(&OptionParameter::Unsigned(0x0040))
    );

    let target_roll = ControllerProgram::parse(
        "duskcontrol 1\nframes 2\n\
             bezier replace from 0 for 1 p0 -127 0 p1 -127 0 p2 -127 0 p3 -127 0\n\
             buttons from 0 for 1 L A\n\
             bezier replace from 1 for 1 p0 0 127 p1 0 127 p2 0 127 p3 0 127\n",
    )
    .unwrap()
    .describe("camera-lock-roll-forward")
    .unwrap();
    assert_eq!(
        target_roll.option.option_type,
        OptionType::Custom("target_roll".into())
    );
}

#[test]
fn existing_plan_types_share_one_adapter_without_changing_realization() {
    let game = GameTacticPlan::new(GameTactic::Interact {
        press_frames: 1,
        recovery_frames: 2,
    });
    let path = MotionPathPlan {
        schema: MOTION_PATH_SCHEMA_V1.into(),
        path: StickPath::Bezier {
            control: [
                StickPoint { x: 0, y: 127 },
                StickPoint { x: 20, y: 100 },
                StickPoint { x: 40, y: 80 },
                StickPoint { x: 60, y: 60 },
            ],
        },
        duration_ticks: 4,
        sample_phase: SamplePhase::default(),
        cancellation_conditions: Vec::new(),
    };
    let native = NativeGenericTacticPlan::new(
        GenericTactic::SeekCoordinate {
            coordinate_f32_bits: [1.0_f32.to_bits(), 2.0_f32.to_bits(), 3.0_f32.to_bits()],
            tolerance_f32_bits: 0.5_f32.to_bits(),
            magnitude: 100,
        },
        8,
    );
    let roll = RollOptionPlan::new(0, 100, 2);
    let controller = ControllerProgram::parse(
            "duskcontrol 1\nframes 4\nbezier replace from 0 for 4 p0 0 127 p1 20 100 p2 40 80 p3 60 60\n",
        )
        .unwrap();

    let game_description = game.describe("interact").unwrap();
    let path_description = path.describe("curve").unwrap();
    let native_description = native.describe("seek").unwrap();
    let roll_description = roll.describe("roll").unwrap();
    let controller_description = controller.describe("controller").unwrap();

    for description in [
        &game_description,
        &path_description,
        &native_description,
        &roll_description,
        &controller_description,
    ] {
        description.validate().unwrap();
        let encoded = serde_json::to_vec(description).unwrap();
        let decoded: TacticAssetDescription = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(&decoded, description);
    }
    assert_eq!(game_description.kind, TacticAssetKind::GameTactic);
    assert_eq!(path_description.option.option_type, OptionType::Bezier);
    assert_eq!(native_description.option.option_type, OptionType::Move);
    assert_eq!(roll_description.option.option_type, OptionType::Roll);
    assert_eq!(
        controller_description.option.option_type,
        OptionType::Custom("reactive_controller".into())
    );
    assert!(game.static_frames().unwrap().is_some());
    let game_exact = game.exact_static_realization("interact").unwrap().unwrap();
    game_exact.validate_against(&game_description).unwrap();
    assert_eq!(
        path.static_frames().unwrap().unwrap(),
        path.realize(None).unwrap().frames
    );
    let path_exact = path.exact_static_realization("curve").unwrap().unwrap();
    path_exact.validate_against(&path_description).unwrap();
    assert!(native.static_frames().unwrap().is_none());
    assert!(native.exact_static_realization("seek").unwrap().is_none());
    let roll_exact = roll.exact_static_realization("roll").unwrap().unwrap();
    roll_exact.validate_against(&roll_description).unwrap();
    assert!(controller.static_frames().unwrap().is_some());
    let controller_exact = controller
        .exact_static_realization("controller")
        .unwrap()
        .unwrap();
    controller_exact
        .validate_against(&controller_description)
        .unwrap();
    assert_eq!(
        controller_exact.execution.emitted_raw_actions,
        controller.static_frames().unwrap().unwrap()
    );
    assert_eq!(game_description.executor, TacticExecutor::StaticPlan);
    assert_eq!(
        native_description.executor,
        TacticExecutor::NativeGenericObservationLoop
    );
    assert_eq!(
        native_description.stopping.termination,
        native.termination_condition().unwrap()
    );
    assert_eq!(
        [
            game_description.content_sha256,
            path_description.content_sha256,
            native_description.content_sha256,
            roll_description.content_sha256,
            controller_description.content_sha256,
        ]
        .into_iter()
        .collect::<BTreeSet<_>>()
        .len(),
        5
    );
}

#[test]
fn reactive_controller_and_native_tactic_declare_exact_observation_families() {
    let controller = ControllerProgram::parse(
            "duskcontrol 1\nframes 3\nseek coordinate replace from 0 for 3 frame world target 1 2 3 offset 0 0 0 magnitude 100 stop 1\n",
        )
        .unwrap();
    let controller_description = controller.describe("seek-world").unwrap();
    assert!(!controller_description.statically_realizable);
    assert_eq!(
        controller_description.executor,
        TacticExecutor::ReactiveControllerProgram
    );
    assert_eq!(
        controller_description.required_observations,
        [
            TacticObservationRequirement::PlayerPosition,
            TacticObservationRequirement::CameraYaw,
        ]
        .into_iter()
        .collect()
    );
    assert!(controller.static_frames().unwrap().is_none());
    assert!(
        controller
            .exact_static_realization("seek-world")
            .unwrap()
            .is_none()
    );

    let exact_actor = ControllerProgram::parse(
            "duskcontrol 1\nframes 3\nseek actor replace from 0 for 3 actor 42 set 7 room 1 stage F_SP103 offset 0 0 0 magnitude 100 stop 1\n",
        )
        .unwrap()
        .describe("seek-exact-actor")
        .unwrap();
    assert_eq!(
        exact_actor.stopping.cancellation,
        vec![OptionCondition::TargetLost {
            target: "controller_exact_actor".into(),
        }]
    );

    let native = NativeGenericTacticPlan::new(
        GenericTactic::SynchronizeButtonEdge {
            button_mask: 0x0100,
            procedure_id: 7,
            animation_resource_id: 12,
            phase_f32_bits: 4.0_f32.to_bits(),
            movement_heading_radians_f32_bits: None,
            movement_magnitude: 0,
        },
        10,
    );
    assert_eq!(
        native.describe("sync").unwrap().required_observations,
        [
            TacticObservationRequirement::PlayerProcedure,
            TacticObservationRequirement::PlayerActionLane,
        ]
        .into_iter()
        .collect()
    );
}

#[test]
fn adapter_rejects_nonportable_option_ids() {
    let tactic = GameTacticPlan::new(GameTactic::Shield { frames: 2 });
    assert_eq!(
        tactic.describe("contains spaces").unwrap_err(),
        TacticAssetError::InvalidOptionId
    );
}

#[test]
fn exact_realization_rejects_descriptor_or_stopping_drift() {
    let tactic = GameTacticPlan::new(GameTactic::Shield { frames: 2 });
    let realization = tactic.exact_static_realization("shield").unwrap().unwrap();
    let mut description = tactic.describe("shield").unwrap();
    description
        .option
        .parameters
        .insert("frames".into(), OptionParameter::Unsigned(3));
    assert!(realization.validate_against(&description).is_err());

    let mut description = tactic.describe("shield").unwrap();
    description.stopping.termination = OptionCondition::TargetReached {
        target: "unrelated".into(),
    };
    assert!(realization.validate_against(&description).is_err());
}

#[test]
fn one_finite_catalog_holds_all_existing_plan_families() {
    let game = TacticCatalogEntry::new(
        "game.interact",
        TacticAssetSource::GameTactic(GameTacticPlan::new(GameTactic::Interact {
            press_frames: 1,
            recovery_frames: 1,
        })),
    )
    .unwrap();
    let path = TacticCatalogEntry::new(
        "path.waypoint",
        TacticAssetSource::MotionPath(MotionPathPlan::new(
            StickPath::Waypoint {
                points: vec![StickPoint { x: 0, y: 100 }],
            },
            2,
        )),
    )
    .unwrap();
    let native = TacticCatalogEntry::new(
        "native.heading",
        TacticAssetSource::NativeGenericTactic(NativeGenericTacticPlan::new(
            GenericTactic::MaintainRelativeHeading {
                heading_radians_f32_bits: 0.0_f32.to_bits(),
                magnitude: 90,
            },
            3,
        )),
    )
    .unwrap();
    let mut roll_plan = RollOptionPlan::new(0, 100, 1);
    roll_plan.button_frame = 1;
    roll_plan.spacing = RollSpacing {
        period_ticks: 4,
        phase_tick: 3,
    };
    let roll = TacticCatalogEntry::new("roll.forward", TacticAssetSource::Roll(roll_plan)).unwrap();
    let controller = TacticCatalogEntry::new(
        "controller.buttons",
        TacticAssetSource::ReactiveController(
            ControllerProgram::parse("duskcontrol 1\nframes 2\nbuttons from 0 for 2 B\n").unwrap(),
        ),
    )
    .unwrap();

    let catalog = TacticAssetCatalog::new(vec![path, native, controller, game, roll]).unwrap();
    assert_eq!(
        catalog
            .entries()
            .iter()
            .map(TacticCatalogEntry::option_id)
            .collect::<Vec<_>>(),
        vec![
            "controller.buttons",
            "game.interact",
            "native.heading",
            "path.waypoint",
            "roll.forward",
        ]
    );
    assert_eq!(catalog.descriptions().len(), 5);
    for option_id in [
        "controller.buttons",
        "game.interact",
        "path.waypoint",
        "roll.forward",
    ] {
        let PreparedTacticExecution::Static(realization) =
            catalog.prepare_execution(option_id).unwrap()
        else {
            panic!("expected exact static execution");
        };
        realization
            .validate_against(catalog.entry(option_id).unwrap().description())
            .unwrap();
    }
    let PreparedTacticExecution::NativeGeneric(native) =
        catalog.prepare_execution("native.heading").unwrap()
    else {
        panic!("expected native generic executor input");
    };
    assert_eq!(native.descriptor().option_id, "native.heading");
    assert_eq!(
        catalog.option_descriptors().count(),
        catalog.entries().len()
    );
    assert_ne!(catalog.action_schema_sha256(), Digest::ZERO);
    assert_eq!(
        catalog.prepare_execution("missing").unwrap_err(),
        TacticAssetError::UnknownOptionId("missing".into())
    );
}

#[test]
fn executable_source_records_round_trip_every_tactic_family_canonically() {
    let mut frame = InputFrame::default();
    frame.owned_ports = 1;
    frame.pads[0].stick_y = 100;
    let sources = vec![
        TacticAssetSource::GameTactic(GameTacticPlan::new(GameTactic::Interact {
            press_frames: 1,
            recovery_frames: 1,
        })),
        TacticAssetSource::NativeGenericTactic(NativeGenericTacticPlan::new(
            GenericTactic::MaintainRelativeHeading {
                heading_radians_f32_bits: 0.0_f32.to_bits(),
                magnitude: 100,
            },
            4,
        )),
        TacticAssetSource::MotionPath(MotionPathPlan::new(
            StickPath::Waypoint {
                points: vec![StickPoint { x: 0, y: 100 }],
            },
            2,
        )),
        TacticAssetSource::Roll(RollOptionPlan::new(0, 100, 2)),
        TacticAssetSource::ReactiveController(
            ControllerProgram::parse(
                "duskcontrol 1\nframes 2\nbezier replace from 0 for 2 p0 0 100 p1 0 100 p2 0 100 p3 0 100\n",
            )
            .unwrap(),
        ),
        TacticAssetSource::RecordedTape(InputTape {
            frames: vec![frame; 2],
            ..InputTape::default()
        }),
    ];

    for source in sources {
        let encoded = EncodedTacticAssetSource::capture(&source).unwrap();
        assert_eq!(encoded.decode().unwrap(), source);
        assert_ne!(encoded.content_sha256().unwrap(), Digest::ZERO);
    }
}

#[test]
fn catalog_dispatches_observation_driven_controller_to_existing_program() {
    let catalog = TacticAssetCatalog::new(vec![
            TacticCatalogEntry::new(
                "controller.seek",
                TacticAssetSource::ReactiveController(
                    ControllerProgram::parse(
                        "duskcontrol 1\nframes 3\nseek coordinate replace from 0 for 3 frame world target 1 2 3 offset 0 0 0 magnitude 100 stop 1\n",
                    )
                    .unwrap(),
                ),
            )
            .unwrap(),
        ])
        .unwrap();

    let PreparedTacticExecution::ReactiveController(program) =
        catalog.prepare_execution("controller.seek").unwrap()
    else {
        panic!("expected reactive controller executor input");
    };
    assert_eq!(program.duration_frames, 3);
}

#[test]
fn recorded_binary_tape_is_an_exact_static_tactic() {
    let mut frame = InputFrame::default();
    frame.owned_ports = 1;
    frame.pads[0].stick_x = 73;
    frame.pads[0].buttons = 0x0100;
    let tape = InputTape {
        frames: vec![frame.clone(), frame, InputFrame::default()],
        ..InputTape::default()
    };
    let entry = TacticCatalogEntry::new(
        "promoted/example",
        TacticAssetSource::RecordedTape(tape.clone()),
    )
    .unwrap();
    assert_eq!(entry.description().kind, TacticAssetKind::RecordedTape);
    assert_eq!(entry.description().option.option_type, OptionType::Roll);
    assert_eq!(entry.description().duration.maximum_ticks, 3);
    assert_eq!(
        entry
            .description()
            .option
            .parameters
            .get("input_tape_sha256"),
        Some(&OptionParameter::Digest(digest(&tape.encode().unwrap())))
    );
    assert_eq!(
        entry
            .description()
            .option
            .parameters
            .get("command_stick_magnitude"),
        Some(&OptionParameter::Unsigned(49))
    );
    assert_eq!(
        entry
            .description()
            .option
            .parameters
            .get("command_button_mask"),
        Some(&OptionParameter::Unsigned(0x0100))
    );
    assert_eq!(
        entry
            .description()
            .option
            .parameters
            .get("command_button_pulse_count"),
        Some(&OptionParameter::Unsigned(1))
    );
    assert!(
        (parameter_f32(entry.description(), "command_button_active_fraction") - 2.0 / 3.0).abs()
            < 1.0e-6
    );
    assert!(
        (parameter_f32(entry.description(), "movement_heading") + std::f32::consts::FRAC_PI_2)
            .abs()
            < 1.0e-6
    );
    assert!(
        (parameter_f32(entry.description(), "command_initial_heading")
            + std::f32::consts::FRAC_PI_2)
            .abs()
            < 1.0e-6
    );
    let catalog = TacticAssetCatalog::new(vec![entry]).unwrap();
    let PreparedTacticExecution::Static(realized) =
        catalog.prepare_execution("promoted/example").unwrap()
    else {
        panic!("recorded tape must use the exact static executor");
    };
    assert_eq!(realized.tape, tape);
    assert_eq!(realized.execution.emitted_raw_actions, tape.frames);
    realized
        .validate_against(catalog.entry("promoted/example").unwrap().description())
        .unwrap();
}

#[test]
fn recorded_tape_summary_captures_turns_and_button_cadence() {
    let frame = |stick_x, stick_y, buttons| {
        let mut frame = InputFrame::default();
        frame.owned_ports = 1;
        frame.pads[0].stick_x = stick_x;
        frame.pads[0].stick_y = stick_y;
        frame.pads[0].buttons = buttons;
        frame
    };
    let tape = InputTape {
        frames: vec![
            frame(0, 127, 0x0100),
            frame(-127, 0, 0),
            frame(-127, 0, 0x0100),
        ],
        ..InputTape::default()
    };

    let description = tape.describe("recorded/turn-and-pulse").unwrap();

    assert_eq!(
        description.option.parameters.get("command_stick_magnitude"),
        Some(&OptionParameter::Unsigned(127))
    );
    assert_eq!(
        description
            .option
            .parameters
            .get("command_button_pulse_count"),
        Some(&OptionParameter::Unsigned(2))
    );
    assert_eq!(
        description.option.parameters.get("button_pulse_phase_tick"),
        Some(&OptionParameter::Unsigned(0))
    );
    assert_eq!(
        parameter_f32(&description, "command_button_mean_interval_ticks"),
        2.0
    );
    assert!(
        (parameter_f32(&description, "command_internal_turn_radians")
            - std::f32::consts::FRAC_PI_2)
            .abs()
            < 1.0e-6
    );
    assert_eq!(parameter_f32(&description, "command_initial_heading"), 0.0);
}

#[test]
fn recorded_tape_uses_the_same_atomic_type_basis_as_live_tactics() {
    let tape = |stick_y, buttons| {
        let mut frame = InputFrame::default();
        frame.owned_ports = 1;
        frame.pads[0].stick_y = stick_y;
        frame.pads[0].buttons = buttons;
        InputTape {
            frames: vec![frame],
            ..InputTape::default()
        }
    };

    assert_eq!(
        tape(127, 0)
            .describe("recorded/move")
            .unwrap()
            .option
            .option_type,
        OptionType::Move
    );
    assert_eq!(
        tape(0, 0)
            .describe("recorded/neutral")
            .unwrap()
            .option
            .option_type,
        OptionType::Neutral
    );
    assert_eq!(
        tape(127, 0x0100)
            .describe("recorded/roll")
            .unwrap()
            .option
            .option_type,
        OptionType::Roll
    );
    assert_eq!(
        tape(0, 0x0200)
            .describe("recorded/prompt")
            .unwrap()
            .option
            .option_type,
        OptionType::Custom("recorded_tape".into())
    );
}

#[test]
fn catalog_rejects_duplicate_concrete_option_identity() {
    let entry = || {
        TacticCatalogEntry::new(
            "duplicate",
            TacticAssetSource::GameTactic(GameTacticPlan::new(GameTactic::Shield { frames: 1 })),
        )
        .unwrap()
    };
    assert_eq!(
        TacticAssetCatalog::new(vec![entry(), entry()]).unwrap_err(),
        TacticAssetError::DuplicateOptionId
    );
}
