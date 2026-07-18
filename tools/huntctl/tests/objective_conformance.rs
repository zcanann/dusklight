use huntctl::harness::objective_suite::{
    ExpectedTerminalClass, OBJECTIVE_SUITE_SCHEMA_V1, ObjectiveBoot, ObjectiveSeed, ObjectiveSuite,
};
use huntctl::learning::offline_rl::movement_action_schema_digest_v2;
use huntctl::milestone_dsl;
use huntctl::tape::{RawPadState, TapeBoot};
use huntctl::{tape_dsl, tape_program::TapeProgram};
use std::fs;
use std::path::PathBuf;

#[test]
fn checked_in_stage_ready_case_is_bound_compilable_and_neutral() {
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let suite: ObjectiveSuite = serde_json::from_slice(
        &fs::read(repository.join("tests/fixtures/automation/objective_conformance_suite.json"))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(suite.schema, OBJECTIVE_SUITE_SCHEMA_V1);
    let report = suite.validate_files(&repository).unwrap();
    assert_eq!(report.case_count, 1);
    assert_eq!(report.positive_count, 1);

    let case = &suite.cases[0];
    assert_eq!(case.id, "stage-ready-f-sp103");
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
