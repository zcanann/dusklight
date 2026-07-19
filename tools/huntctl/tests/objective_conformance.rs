use huntctl::controller_program::{ControllerProgram, Operation};
use huntctl::harness::objective_suite::{
    ExpectedTerminalClass, OBJECTIVE_SUITE_SCHEMA_V2, ObjectiveBoot, ObjectiveCaseRole,
    ObjectiveSeed, ObjectiveSuite,
};
use huntctl::learning::offline_rl::movement_action_schema_digest_v2;
use huntctl::milestone_dsl;
use huntctl::tape::{RawPadState, TapeBoot};
use huntctl::{tape_dsl, tape_program::TapeProgram};
use std::fs;
use std::path::PathBuf;

fn checked_suite() -> (PathBuf, ObjectiveSuite) {
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let suite: ObjectiveSuite = serde_json::from_slice(
        &fs::read(repository.join("tests/fixtures/automation/objective_conformance_suite.json"))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(suite.schema, OBJECTIVE_SUITE_SCHEMA_V2);
    let report = suite.validate_files(&repository).unwrap();
    assert_eq!(report.case_count, 8);
    assert_eq!(report.positive_count, 4);
    assert_eq!(report.negative_control_count, 4);
    (repository, suite)
}

#[test]
fn checked_in_stage_ready_case_is_bound_compilable_and_neutral() {
    let (repository, suite) = checked_suite();
    let case = suite
        .cases
        .iter()
        .find(|case| case.id == "stage-ready-f-sp103")
        .unwrap();
    assert_eq!(case.expected_terminal, ExpectedTerminalClass::Reached);
    assert_eq!(case.repetitions, 2);
    assert_eq!(
        case.action_schema.sha256,
        movement_action_schema_digest_v2()
    );
    let ObjectiveBoot::Stage {
        stage,
        room,
        point,
        layer,
        save_slot,
    } = &case.boot
    else {
        panic!("stage-ready case must use direct stage boot");
    };

    let objective_source =
        fs::read_to_string(repository.join(&case.objective.source.path)).unwrap();
    let objective = milestone_dsl::parse(&objective_source).unwrap();
    assert_eq!(objective.definitions.len(), 1);
    assert_eq!(objective.definitions[0].name, "stage_ready");
    assert_eq!(objective.definitions[0].stable_ticks, 3);

    let ObjectiveSeed::TapeSource { artifact } = &case.seed else {
        panic!("stage-ready case must retain an authored tape seed");
    };
    let tape_source = fs::read_to_string(repository.join(&artifact.path)).unwrap();
    let program: TapeProgram = tape_dsl::parse(&tape_source).unwrap();
    let tape = program.compile().unwrap().tape;
    assert_eq!(
        tape.boot,
        TapeBoot::Stage {
            stage: stage.clone(),
            room: *room,
            point: *point,
            layer: *layer,
            save_slot: *save_slot,
            fixture: None,
        }
    );
    assert_eq!(tape.frames.len(), case.logical_tick_budget as usize);
    assert!(
        tape.frames
            .iter()
            .all(|frame| { frame.pads.iter().all(|pad| *pad == RawPadState::default()) })
    );
}

#[test]
fn checked_in_reach_point_case_moves_to_a_stable_bounded_region() {
    let (repository, suite) = checked_suite();
    let case = suite
        .cases
        .iter()
        .find(|case| case.id == "reach-point-ordon-ranch")
        .unwrap();
    assert_eq!(case.expected_terminal, ExpectedTerminalClass::Reached);
    assert_eq!(case.repetitions, 2);
    assert_eq!(
        case.action_schema.sha256,
        movement_action_schema_digest_v2()
    );
    assert_eq!(
        case.observation_requirements.facts,
        [
            "player.exists",
            "player.in_aabb",
            "player.is_link",
            "stage.name",
            "stage.room",
        ]
    );

    let ObjectiveBoot::Stage {
        stage,
        room,
        point,
        layer,
        save_slot,
    } = &case.boot
    else {
        panic!("reach-point case must use direct stage boot");
    };
    let objective_source =
        fs::read_to_string(repository.join(&case.objective.source.path)).unwrap();
    let objective = milestone_dsl::parse(&objective_source).unwrap();
    assert_eq!(objective.definitions.len(), 1);
    assert_eq!(objective.definitions[0].name, "reach_point_ordon");
    assert_eq!(objective.definitions[0].stable_ticks, 5);
    assert!(objective_source.contains("player.in_aabb(-1700.0, 100.0, -9150.0"));

    let ObjectiveSeed::TapeSource { artifact } = &case.seed else {
        panic!("reach-point case must retain an authored tape seed");
    };
    let tape_source = fs::read_to_string(repository.join(&artifact.path)).unwrap();
    let program: TapeProgram = tape_dsl::parse(&tape_source).unwrap();
    let tape = program.compile().unwrap().tape;
    assert_eq!(
        tape.boot,
        TapeBoot::Stage {
            stage: stage.clone(),
            room: *room,
            point: *point,
            layer: *layer,
            save_slot: *save_slot,
            fixture: None,
        }
    );
    assert_eq!(tape.frames.len(), case.logical_tick_budget as usize);
    assert!(
        tape.frames
            .iter()
            .any(|frame| { frame.pads[0].stick_x != 0 || frame.pads[0].stick_y != 0 })
    );
}

#[test]
fn every_positive_has_a_cheap_negative_control() {
    let (repository, suite) = checked_suite();
    for (positive_id, control_id) in [
        ("reach-point-ordon-ranch", "reach-point-ordon-ranch-neutral"),
        ("stage-ready-f-sp103", "stage-ready-f-sp103-wrong-stage"),
        ("talk-to-aru", "talk-to-aru-no-interaction"),
        ("pick-up-stone", "pick-up-stone-no-interaction"),
    ] {
        let positive = suite
            .cases
            .iter()
            .find(|case| case.id == positive_id)
            .unwrap();
        let control = suite
            .cases
            .iter()
            .find(|case| case.id == control_id)
            .unwrap();
        assert_eq!(positive.role, ObjectiveCaseRole::Positive);
        assert_eq!(control.role, ObjectiveCaseRole::NegativeControl);
        assert_eq!(control.control_for.as_deref(), Some(positive_id));
        assert_eq!(
            control.expected_terminal,
            ExpectedTerminalClass::ObjectiveMiss
        );
        assert_eq!(control.objective, positive.objective);
        assert_eq!(control.action_schema, positive.action_schema);
        assert_eq!(
            control.observation_requirements,
            positive.observation_requirements
        );
        assert_eq!(control.logical_tick_budget, positive.logical_tick_budget);
        assert_eq!(control.repetitions, positive.repetitions);

        match (&positive.seed, &control.seed) {
            (ObjectiveSeed::TapeSource { .. }, ObjectiveSeed::TapeSource { artifact }) => {
                let source = fs::read_to_string(repository.join(&artifact.path)).unwrap();
                let tape = tape_dsl::parse(&source).unwrap().compile().unwrap().tape;
                assert_eq!(tape.frames.len(), control.logical_tick_budget as usize);
                assert!(
                    tape.frames.iter().all(|frame| {
                        frame.pads.iter().all(|pad| *pad == RawPadState::default())
                    })
                );
            }
            (
                ObjectiveSeed::Controller {
                    artifact: positive_artifact,
                },
                ObjectiveSeed::Controller {
                    artifact: control_artifact,
                },
            ) => {
                let positive_program = ControllerProgram::decode(
                    &fs::read(repository.join(&positive_artifact.path)).unwrap(),
                )
                .unwrap();
                let control_program = ControllerProgram::decode(
                    &fs::read(repository.join(&control_artifact.path)).unwrap(),
                )
                .unwrap();
                assert_eq!(
                    u64::from(positive_program.duration_frames),
                    positive.logical_tick_budget
                );
                assert_eq!(
                    u64::from(control_program.duration_frames),
                    control.logical_tick_budget
                );
                assert!(
                    positive_program
                        .layers
                        .iter()
                        .any(|layer| matches!(layer.operation, Operation::Buttons { .. }))
                );
                assert!(
                    control_program
                        .layers
                        .iter()
                        .all(|layer| !matches!(layer.operation, Operation::Buttons { .. }))
                );
            }
            _ => panic!("positive and control seeds must use the same input representation"),
        }
    }
}

#[test]
fn interaction_cases_bind_exact_real_placements_and_player_action_v3() {
    let (repository, suite) = checked_suite();
    for (case_id, goal, identity_fragments) in [
        (
            "talk-to-aru",
            "talk_to_aru",
            [
                "talk_partner.actor_name == 579",
                "talk_partner.set_id == 65535",
                "talk_partner.home_position.x between 448.79 and 448.80",
            ],
        ),
        (
            "pick-up-stone",
            "pick_up_stone",
            [
                "grabbed_actor.actor_name == 765",
                "grabbed_actor.set_id == 65535",
                "grabbed_actor.home_position.x between 1073.22 and 1073.24",
            ],
        ),
    ] {
        let case = suite.cases.iter().find(|case| case.id == case_id).unwrap();
        assert_eq!(case.objective.goal, goal);
        assert_eq!(case.expected_terminal, ExpectedTerminalClass::Reached);
        assert!(
            case.observation_requirements
                .families
                .iter()
                .any(|family| { family.id == "player_action" && family.minimum_version == 3 })
        );
        let source = fs::read_to_string(repository.join(&case.objective.source.path)).unwrap();
        for fragment in identity_fragments {
            assert!(
                source.contains(fragment),
                "{case_id} is missing {fragment:?}"
            );
        }
    }
}
