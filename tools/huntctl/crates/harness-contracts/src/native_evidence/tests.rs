use super::*;
use dusklight_automation_contracts::artifact::Digest;

fn tape_source() -> ObjectiveSeed {
    ObjectiveSeed::TapeSource {
        artifact: ArtifactReference {
            path: "inputs/eye-shredder.tas".into(),
            sha256: Digest([7; 32]),
        },
    }
}

#[test]
fn eye_shredder_is_closed_to_process_booted_tapes() {
    let evidence = HarnessNativeEvidenceRequest::EyeShredderV4;
    assert!(
        evidence
            .validate_for(&ObjectiveBoot::Process, &tape_source())
            .is_ok()
    );
    assert!(
        evidence
            .validate_for(
                &ObjectiveBoot::Stage {
                    stage: "F_SP103".into(),
                    room: 0,
                    point: 0,
                    layer: 0,
                    save_slot: None,
                },
                &tape_source(),
            )
            .is_err()
    );
    assert!(
        evidence
            .validate_for(&ObjectiveBoot::Process, &ObjectiveSeed::Neutral)
            .is_err()
    );
}
