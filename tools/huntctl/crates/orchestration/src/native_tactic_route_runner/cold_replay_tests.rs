use super::*;

fn digest(value: u8) -> Digest {
    Digest([value; 32])
}

fn artifact(path: &str, value: u8) -> NativeTacticColdReplayArtifact {
    NativeTacticColdReplayArtifact {
        path: path.into(),
        sha256: digest(value),
    }
}

fn execution_artifact(path: &str, value: u8) -> ArtifactReference {
    ArtifactReference {
        path: path.into(),
        sha256: digest(value),
    }
}

fn fingerprint(value: char) -> BoundaryFingerprint {
    BoundaryFingerprint {
        schema: "dusklight.milestone-boundary/v6".into(),
        algorithm: "xxh3-128".into(),
        canonical_encoding: "little-endian-fixed-v6".into(),
        digest: std::iter::repeat(value).take(32).collect(),
    }
}

fn attempt(repetition: u32) -> NativeTacticColdReplayAttempt {
    NativeTacticColdReplayAttempt {
        repetition,
        controller_tape: artifact(&format!("repeat-{repetition:03}/controller.tape"), 10),
        milestone_result: artifact(
            &format!("repeat-{repetition:03}/milestones.json"),
            20 + repetition as u8,
        ),
        stdout: artifact(
            &format!("repeat-{repetition:03}/stdout.txt"),
            30 + repetition as u8,
        ),
        stderr: artifact(
            &format!("repeat-{repetition:03}/stderr.txt"),
            40 + repetition as u8,
        ),
        sim_tick: 512,
        tape_frame: 12,
        boundary_index: 13,
        first_hit_tick: 2,
        boundary_fingerprint: fingerprint('a'),
    }
}

pub(crate) fn proof() -> NativeTacticColdReplayProof {
    let mut proof = NativeTacticColdReplayProof {
        schema: NATIVE_TACTIC_COLD_REPLAY_PROOF_SCHEMA_V1.into(),
        content_sha256: Digest::ZERO,
        optimization_request_sha256: digest(1),
        execution_binding_sha256: digest(2),
        execution_plan_sha256: digest(3),
        route_report_sha256: digest(4),
        seed: 155_921,
        state_graph_sha256: digest(5),
        terminal_result_sha256: digest(6),
        terminal_state_sha256: digest(7),
        objective_sha256: digest(8),
        source_boundary_index: 10,
        source_boundary_fingerprint: "11111111111111111111111111111111".into(),
        native_source_boundary_fingerprint: "22222222222222222222222222222222".into(),
        goal: "ordon-load-zone".into(),
        terminal_program_sha256: digest(9),
        terminal_definition_sha256: digest(8),
        first_hit_tick: 2,
        maximum_first_hit_tick: 123,
        controller_tape: artifact(NATIVE_TACTIC_COLD_REPLAY_TAPE_FILE, 10),
        controller_tape_frames: 13,
        executable: execution_artifact("build/dusklight.exe", 11),
        runtime_dependencies: vec![execution_artifact("build/runtime.dll", 12)],
        game_data: execution_artifact("game.iso", 13),
        milestone_program: execution_artifact("goals.json", 14),
        world_context: execution_artifact("world.json", 15),
        card_fixture_manifest: execution_artifact("fixture.json", 16),
        fidelity: NativeTacticColdReplayFidelity::exact_headless(),
        controller_in_loop: false,
        learner_in_loop: false,
        attempts: vec![attempt(1), attempt(2)],
    };
    proof.content_sha256 = proof.identity().unwrap();
    proof
}

fn reseal(proof: &mut NativeTacticColdReplayProof) {
    proof.content_sha256 = Digest::ZERO;
    proof.content_sha256 = proof.identity().unwrap();
}

#[test]
fn exact_repeated_cold_replay_proof_is_accepted() {
    proof().validate_shape().unwrap();
}

#[test]
fn controller_or_learner_authority_is_rejected() {
    let mutations: [fn(&mut NativeTacticColdReplayProof); 2] = [
        |proof: &mut NativeTacticColdReplayProof| proof.controller_in_loop = true,
        |proof: &mut NativeTacticColdReplayProof| proof.learner_in_loop = true,
    ];
    for mutate in mutations {
        let mut proof = proof();
        mutate(&mut proof);
        reseal(&mut proof);
        assert!(proof.validate_shape().is_err());
    }
}

#[test]
fn differing_terminal_evidence_is_rejected() {
    let mut proof = proof();
    proof.attempts[1].boundary_fingerprint = fingerprint('b');
    reseal(&mut proof);
    assert!(proof.validate_shape().is_err());
}

#[test]
fn single_repetition_or_relaxed_fidelity_is_rejected() {
    let mut single = proof();
    single.attempts.pop();
    reseal(&mut single);
    assert!(single.validate_shape().is_err());

    let mut relaxed = proof();
    relaxed.fidelity.unpaced = false;
    reseal(&mut relaxed);
    assert!(relaxed.validate_shape().is_err());
}

#[test]
fn tape_length_and_tick_ceiling_are_exact() {
    let mut wrong_length = proof();
    wrong_length.controller_tape_frames += 1;
    reseal(&mut wrong_length);
    assert!(wrong_length.validate_shape().is_err());

    let mut too_slow = proof();
    too_slow.maximum_first_hit_tick = 1;
    reseal(&mut too_slow);
    assert!(too_slow.validate_shape().is_err());
}
