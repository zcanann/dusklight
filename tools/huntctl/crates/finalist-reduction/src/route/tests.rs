use super::*;
use dusklight_automation_contracts::tape::InputFrame;
use dusklight_search::search::SearchPadState;

fn candidate() -> Candidate {
    Candidate {
        schema: "dusklight-search-candidate/v2".into(),
        segment: SegmentProfile::Fsp103ToFsp104,
        boot: TapeBoot::Process,
        actions: vec![
            MacroAction::PadRun {
                pad: SearchPadState {
                    buttons: 0,
                    stick_x: 127,
                    stick_y: 0,
                    substick_x: 0,
                    substick_y: 0,
                    trigger_left: 0,
                    trigger_right: 0,
                    analog_a: 0,
                    analog_b: 0,
                    connected: true,
                    error: 0,
                },
                frames: 4,
                imported_owned_ports: None,
                port_one_secondary_pads: None,
            },
            MacroAction::Neutral { frames: 3 },
            MacroAction::Move {
                angle_degrees: 90,
                magnitude: 127,
                frames: 2,
            },
        ],
        ancestry: Ancestry::default(),
    }
}

fn objective() -> AnchoredObjectiveIdentity {
    AnchoredObjectiveIdentity {
        schema: "dusklight-anchored-search-objective/v2".into(),
        segment: SegmentProfile::Fsp103ToFsp104,
        digest: "a".repeat(64),
        prefix_sha256: "b".repeat(64),
        prefix_frames: 440,
        milestone_program_sha256: "c".repeat(64),
        game_sha256: "d".repeat(64),
        dvd_sha256: "e".repeat(64),
        source_milestone: "source".into(),
        source_definition_sha256: "f".repeat(64),
        source_boundary_fingerprint: "1".repeat(32),
        source_tape_frame: 439,
        source_boundary_index: 440,
        goal_milestone: "goal".into(),
        goal_definition_sha256: "2".repeat(64),
    }
}

fn config(candidate: Candidate) -> AnchoredRouteMinimizeConfig {
    AnchoredRouteMinimizeConfig {
        candidate,
        objective: AnchoredObjectiveConfig {
            segment: SegmentProfile::Fsp103ToFsp104,
            prefix_tape: "prefix.tape".into(),
            milestone_program: "objective.dmsp".into(),
            game: "game".into(),
            dvd: "dvd".into(),
            source_milestone: "source".into(),
            source_boundary_fingerprint: "1".repeat(32),
            goal_milestone: "goal".into(),
        },
        output_root: "output".into(),
        working_directory: ".".into(),
        game_args_prefix: Vec::new(),
        workers: 1,
        repetitions: 2,
        candidate_budget: 10,
        resume: true,
        timeout: std::time::Duration::from_secs(1),
        harness: None,
    }
}

#[test]
fn partitions_and_duration_reductions_are_bounded_and_deterministic() {
    let source = candidate();
    source.validate().unwrap();
    let partitions = action_partition_removals(&source, 2, 1).unwrap();
    assert_eq!(partitions.len(), 2);
    assert!(
        partitions
            .iter()
            .all(|candidate| candidate.actions.len() < 3)
    );
    let reductions = duration_reductions(&source, 2).unwrap();
    assert_eq!(reductions.len(), 3);
    assert!(
        reductions
            .iter()
            .all(|candidate| candidate.frame_count() == source.frame_count() - 1)
    );
}

#[test]
fn route_input_golf_preserves_movement_while_editing_button_pulses() {
    let disconnected = RawPadState {
        connected: false,
        error: -1,
        ..RawPadState::default()
    };
    let mut tape = InputTape {
        boot: TapeBoot::Process,
        frames: vec![
            InputFrame {
                owned_ports: 0x01,
                pads: [
                    RawPadState::default(),
                    disconnected,
                    disconnected,
                    disconnected
                ],
                ..InputFrame::default()
            };
            10
        ],
        ..InputTape::default()
    };
    tape.frames[3].pads[0].buttons = BUTTON_A;
    tape.frames[3].pads[0].stick_x = 40;
    tape.frames[3].pads[0].connected = true;
    tape.frames[6].pads[0].stick_y = 50;
    tape.frames[6].pads[0].connected = true;
    tape.frames[7].pads[0].buttons = BUTTON_START;
    tape.frames[7].pads[0].stick_x = 60;
    tape.frames[7].pads[0].connected = true;
    // A non-menu input is deliberately outside the edit surface.
    tape.frames[9].pads[0].buttons = 0x0200;
    let candidate = Candidate::from_absolute_tape(SegmentProfile::Fsp103ToFsp104, &tape).unwrap();
    let proven = ProvenRouteCandidate {
        tape,
        candidate,
        first_hit_tick: 10,
        goal_sim_tick: 10,
        goal_tape_frame: 9,
        goal_boundary_fingerprint: BoundaryFingerprint {
            schema: "dusklight.milestone-boundary/v4".into(),
            algorithm: "xxh3-128".into(),
            canonical_encoding: "little-endian-fixed-v4".into(),
            digest: "1".repeat(32),
        },
    };

    let proposals = input_golf_proposals(&proven, 1, 6).unwrap();
    assert_eq!(proposals.len(), 6);
    assert_eq!(
        button_pulse_timestamps(&proposals[0].compile().unwrap()).unwrap(),
        vec![7]
    );
    assert_eq!(
        proposals[0].compile().unwrap().frames[3].pads[0].stick_x,
        40
    );
    assert_eq!(
        button_pulse_timestamps(&proposals[1].compile().unwrap()).unwrap(),
        vec![3]
    );
    assert_eq!(
        button_pulse_timestamps(&proposals[2].compile().unwrap()).unwrap(),
        vec![3, 7]
    );
    assert_eq!(
        proposals[2].compile().unwrap().frames[3].pads[0].buttons,
        BUTTON_START
    );
    assert_eq!(
        proposals[2].compile().unwrap().frames[3].pads[0].stick_x,
        40
    );
    assert_eq!(
        proposals[3].compile().unwrap().frames[7].pads[0].buttons,
        BUTTON_A
    );
    assert_eq!(
        proposals[4].compile().unwrap().frames[9].pads[0].buttons,
        0x0200
    );
    let swapped = proposals[5].compile().unwrap();
    assert_eq!(button_pulse_timestamps(&swapped).unwrap(), vec![3, 6]);
    assert_eq!(swapped.frames[6].pads[0].buttons, BUTTON_A);
    assert_eq!(swapped.frames[6].pads[0].stick_y, 50);
    assert_eq!(swapped.frames[7].pads[0].stick_x, 60);
    assert_eq!(swapped.frames[9].pads[0].buttons, 0x0200);
}

#[test]
fn route_input_quality_prefers_goal_tick_then_simplicity_then_earlier_pulses() {
    let make = |tick: u64, pulse_frames: &[usize]| {
        let mut tape = InputTape {
            frames: vec![
                InputFrame {
                    owned_ports: 0x0f,
                    ..InputFrame::default()
                };
                12
            ],
            ..InputTape::default()
        };
        for frame in pulse_frames {
            tape.frames[*frame].pads[0].buttons = BUTTON_A;
        }
        let candidate = Candidate::from_absolute_tape(SegmentProfile::BootToFsp103, &tape).unwrap();
        ProvenRouteCandidate {
            candidate,
            tape,
            first_hit_tick: tick,
            goal_sim_tick: tick,
            goal_tape_frame: 11,
            goal_boundary_fingerprint: BoundaryFingerprint {
                schema: "dusklight.milestone-boundary/v4".into(),
                algorithm: "xxh3-128".into(),
                canonical_encoding: "little-endian-fixed-v4".into(),
                digest: "1".repeat(32),
            },
        }
    };
    assert!(
        input_golf_quality(&make(9, &[5, 8])).unwrap()
            < input_golf_quality(&make(10, &[1])).unwrap()
    );
    assert!(
        input_golf_quality(&make(10, &[5])).unwrap()
            < input_golf_quality(&make(10, &[2, 5])).unwrap()
    );
    assert!(
        input_golf_quality(&make(10, &[4, 7])).unwrap()
            < input_golf_quality(&make(10, &[5, 7])).unwrap()
    );
}

#[test]
fn exact_target_rejects_tick_or_terminal_state_drift() {
    let boundary = BoundaryFingerprint {
        schema: "dusklight.milestone-boundary/v4".into(),
        algorithm: "xxh3-128".into(),
        canonical_encoding: "little-endian-fixed-v4".into(),
        digest: "11111111111111111111111111111111".into(),
    };
    let proven = ProvenRouteCandidate {
        tape: candidate().compile().unwrap(),
        candidate: candidate(),
        first_hit_tick: 10,
        goal_sim_tick: 450,
        goal_tape_frame: 450,
        goal_boundary_fingerprint: boundary.clone(),
    };
    let target = RouteReductionTarget {
        first_hit_tick: 10,
        goal_sim_tick: 450,
        goal_tape_frame: 450,
        goal_boundary_fingerprint: boundary,
    };
    assert!(target.accepts(&proven));
    let mut drifted = proven.clone();
    drifted.first_hit_tick += 1;
    assert!(!target.accepts(&drifted));
    drifted = proven;
    drifted.goal_boundary_fingerprint.digest = "22222222222222222222222222222222".into();
    assert!(!target.accepts(&drifted));
}

#[test]
fn resume_checkpoint_rejects_budget_history_and_target_drift() {
    let retained = candidate();
    let retained_tape = retained.compile().unwrap();
    let objective = objective();
    let target = RouteReductionTarget {
        first_hit_tick: 10,
        goal_sim_tick: 575,
        goal_tape_frame: 575,
        goal_boundary_fingerprint: BoundaryFingerprint {
            schema: "dusklight.milestone-boundary/v4".into(),
            algorithm: "xxh3-128".into(),
            canonical_encoding: "little-endian-fixed-v4".into(),
            digest: "3".repeat(32),
        },
    };
    let source_id = retained.id().unwrap();
    let checkpoint = RouteMinimizeCheckpoint {
        schema: "dusklight-anchored-route-minimization-checkpoint/v1".into(),
        objective: objective.clone(),
        harness_request_sha256: None,
        source_candidate_id: source_id.clone(),
        candidate_budget: 10,
        target: target.clone(),
        retained_candidate: retained.clone(),
        history: vec![AnchoredRouteMinimizeRound {
            round: 1,
            operation: "trim_after_goal".into(),
            evaluated_candidates: 1,
            accepted_candidate_id: Some(source_id.clone()),
            retained_frames: retained.frame_count(),
            retained_actions: retained.actions.len(),
            retained_input_complexity: tape_input_complexity(&retained_tape),
        }],
        proposal_evaluations: 1,
        accepted_reductions: 1,
        next_round: 2,
        phase: RouteMinimizePhase::Actions { granularity: 2 },
    };
    let config = config(retained);
    validate_checkpoint(&config, &objective, &source_id, &target, &checkpoint).unwrap();

    let mut tampered = checkpoint.clone();
    tampered.proposal_evaluations = 2;
    assert!(validate_checkpoint(&config, &objective, &source_id, &target, &tampered).is_err());
    tampered = checkpoint.clone();
    tampered.target.goal_sim_tick += 1;
    assert!(validate_checkpoint(&config, &objective, &source_id, &target, &tampered).is_err());
    tampered = checkpoint.clone();
    let MacroAction::Move { angle_degrees, .. } = &mut tampered.retained_candidate.actions[2]
    else {
        panic!("fixture action changed")
    };
    *angle_degrees += 1;
    assert!(validate_checkpoint(&config, &objective, &source_id, &target, &tampered).is_err());
    tampered = checkpoint.clone();
    tampered.history[0].operation = "reduce_action_duration".into();
    assert!(validate_checkpoint(&config, &objective, &source_id, &target, &tampered).is_err());
    let mut changed_budget = config;
    changed_budget.candidate_budget = 11;
    assert!(
        validate_checkpoint(
            &changed_budget,
            &objective,
            &source_id,
            &target,
            &checkpoint,
        )
        .is_err()
    );
}

#[test]
fn checkpoint_authority_rejects_legacy_or_changed_run_requests() {
    let request = ArtifactDigest([1; 32]);
    let changed = ArtifactDigest([2; 32]);
    assert!(checkpoint_harness_is_valid(
        "dusklight-anchored-route-minimization-checkpoint/v1",
        None,
        None,
    ));
    assert!(!checkpoint_harness_is_valid(
        "dusklight-anchored-route-minimization-checkpoint/v1",
        None,
        Some(request),
    ));
    assert!(checkpoint_harness_is_valid(
        "dusklight-anchored-route-minimization-checkpoint/v2",
        Some(request),
        Some(request),
    ));
    assert!(!checkpoint_harness_is_valid(
        "dusklight-anchored-route-minimization-checkpoint/v2",
        Some(request),
        Some(changed),
    ));
}
