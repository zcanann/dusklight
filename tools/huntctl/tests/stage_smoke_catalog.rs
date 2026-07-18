use huntctl::tape::TapeBoot;
use huntctl::{tape_dsl, tape_program::TapeProgram};
use serde::Deserialize;
use std::fs;
use std::path::PathBuf;

#[derive(Deserialize)]
struct Catalog {
    schema: String,
    entries: Vec<Entry>,
}

#[derive(Deserialize)]
struct Entry {
    id: String,
    source: PathBuf,
    boot: TapeBoot,
    readiness_milestone: String,
    expected: Expected,
    repeat_proof_count: u32,
}

#[derive(Deserialize)]
struct Expected {
    boundary_schema: String,
    canonical_encoding: String,
    algorithm: String,
    digest: String,
    sim_tick: u64,
    tape_frame: u64,
}

#[test]
fn checked_in_stage_smoke_catalog_has_exact_compilable_origins() {
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let catalog: Catalog = serde_json::from_slice(
        &fs::read(repository.join("tests/fixtures/automation/stage_smoke_catalog.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(catalog.schema, "dusklight-stage-smoke-catalog/v1");
    assert!(!catalog.entries.is_empty());
    for entry in catalog.entries {
        assert!(!entry.id.is_empty());
        assert!(!entry.readiness_milestone.is_empty());
        assert!(entry.repeat_proof_count >= 2);
        assert_eq!(
            entry.expected.boundary_schema,
            "dusklight.milestone-boundary/v4"
        );
        assert_eq!(entry.expected.canonical_encoding, "little-endian-fixed-v4");
        assert_eq!(entry.expected.algorithm, "xxh3-128");
        assert_eq!(entry.expected.digest.len(), 32);
        assert!(
            entry
                .expected
                .digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        );
        assert_eq!(entry.expected.sim_tick, entry.expected.tape_frame);

        let source = fs::read_to_string(repository.join(entry.source)).unwrap();
        let program: TapeProgram = tape_dsl::parse(&source).unwrap();
        let tape = program.compile().unwrap().tape;
        assert_eq!(tape.boot, entry.boot);
        assert!(!tape.frames.is_empty());
    }
}
