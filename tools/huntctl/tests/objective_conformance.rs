use huntctl::artifact::Digest;
use huntctl::milestone_dsl;
use huntctl::objective_conformance::{
    ConformanceBoot, ConformanceSeed, OBJECTIVE_CONFORMANCE_SUITE_SCHEMA_V1,
    ObjectiveConformanceSuite,
};
use huntctl::scenario_fixture::ScenarioFixture;
use huntctl::tape::{RawPadState, TapeBoot};
use huntctl::{tape_dsl, tape_program::TapeProgram};
use sha2::{Digest as _, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn authenticated_bytes(repository: &Path, path: &Path, expected: Digest) -> Vec<u8> {
    let bytes = fs::read(repository.join(path)).unwrap();
    assert_eq!(Digest(Sha256::digest(&bytes).into()), expected);
    bytes
}

#[test]
fn checked_in_stage_ready_case_is_content_authenticated_and_neutral() {
    let repository = repository();
    let suite: ObjectiveConformanceSuite = serde_json::from_slice(
        &fs::read(repository.join("tests/fixtures/automation/objective_conformance_suite.json"))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(suite.schema, OBJECTIVE_CONFORMANCE_SUITE_SCHEMA_V1);
    suite.validate().unwrap();
    assert_eq!(suite.cases.len(), 1);

    let case = &suite.cases[0];
    assert_eq!(case.id, "stage-ready-f-sp103");
    let ConformanceBoot::Stage {
        stage,
        room,
        point,
        layer,
        save_slot,
    } = &case.boot
    else {
        panic!("stage-ready case must use direct stage boot");
    };
    assert_eq!(
        (stage.as_str(), *room, *point, *layer, *save_slot),
        ("F_SP103", 1, 1, 3, None)
    );

    let fixture = case.scenario_fixture.as_ref().unwrap();
    let fixture: ScenarioFixture = serde_json::from_slice(&authenticated_bytes(
        &repository,
        &fixture.path,
        fixture.sha256,
    ))
    .unwrap();
    fixture.validate().unwrap();

    let objective_source = String::from_utf8(authenticated_bytes(
        &repository,
        &case.objective_program.path,
        case.objective_program.sha256,
    ))
    .unwrap();
    let objective = milestone_dsl::parse(&objective_source).unwrap();
    assert_eq!(objective.definitions.len(), 1);
    assert_eq!(objective.definitions[0].name, "stage_ready");
    assert_eq!(objective.definitions[0].stable_ticks, 3);
    milestone_dsl::compile(&objective).unwrap();

    let ConformanceSeed::Tape { artifact } = &case.seed else {
        panic!("stage-ready case must retain an absolute tape seed");
    };
    let tape_source = String::from_utf8(authenticated_bytes(
        &repository,
        &artifact.path,
        artifact.sha256,
    ))
    .unwrap();
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
    assert_eq!(tape.frames.len(), case.budget.logical_ticks as usize);
    assert!(
        tape.frames
            .iter()
            .all(|frame| { frame.pads.iter().all(|pad| *pad == RawPadState::default()) })
    );
}
