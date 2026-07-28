use super::*;
use dusklight_automation_contracts::tape::RawPadState;
use dusklight_world::stage_boot_catalog::{
    BootLayerSource, BootLayerSourceKind, BootPointSource, BootPointSourceKind,
    STAGE_BOOT_CATALOG_SCHEMA, StageCatalogStatus, StageInventoryStatus,
};
use std::time::{SystemTime, UNIX_EPOCH};

fn digest(byte: u8) -> Digest {
    Digest([byte; 32])
}

fn catalog() -> StageBootCatalog {
    let candidates = [0_i16, 1]
        .into_iter()
        .map(|point| StageBootCandidate {
            id: format!("F_SP103/room/0/point/{point}/layer/-1"),
            stage: "F_SP103".into(),
            room: 0,
            point,
            layer: -1,
            point_sources: vec![BootPointSource {
                kind: BootPointSourceKind::RetailPlayerSpawn,
                stable_id: Some(format!("spawn-{point}")),
            }],
            layer_sources: vec![BootLayerSource {
                kind: BootLayerSourceKind::ResolvedDefault,
                chunk_tag: None,
            }],
        })
        .collect::<Vec<_>>();
    StageBootCatalog {
        schema: STAGE_BOOT_CATALOG_SCHEMA.into(),
        known_loader_sha256: None,
        stages: vec![StageCatalogStatus {
            stage: "F_SP103".into(),
            resources_present: true,
            inventory_status: StageInventoryStatus::Complete,
            inventory_sha256: Some(digest(7)),
            diagnostic: None,
            room_count: 1,
            player_spawn_count: 2,
            candidate_count: 2,
        }],
        candidates,
    }
}

fn ledger(catalog: &StageBootCatalog, maximum_attempts: u8) -> StageSurveyLedger {
    StageSurveyLedger::new(
        catalog,
        StageSurveyIdentity {
            catalog_sha256: catalog.digest().unwrap(),
            executable_sha256: digest(1),
            game_data_sha256: digest(2),
            card_fixture_sha256: digest(3),
            observation_schema_sha256: digest(4),
            settings_sha256: digest(5),
        },
        StageSurveyPolicy {
            probe_ticks: 30,
            probe: StageSurveyProbeKind::Neutral,
            native_stage_readiness_ticks: DEFAULT_NATIVE_STAGE_READINESS_TICKS,
            host_timeout_millis: 120_000,
            maximum_attempts_per_case: maximum_attempts,
            fidelity_profile: STAGE_SURVEY_FIDELITY.into(),
        },
    )
    .unwrap()
}

fn temporary_root(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "dusklight-stage-survey-{label}-{}-{nonce}",
        std::process::id()
    ))
}

fn failed(outcome: StageSurveyAttemptOutcome) -> StageSurveyAttempt {
    StageSurveyAttempt {
        number: 0,
        outcome,
        exit_code: (outcome != StageSurveyAttemptOutcome::HostTimeout
            && outcome != StageSurveyAttemptOutcome::LaunchFailure)
            .then_some(1),
        elapsed_millis: 50,
        observation_sha256: None,
        actor_catalog_sha256: None,
        observed_actor_count: None,
        retained_actor_count: None,
        actor_catalog_truncated: None,
        state_sequence_sha256: None,
        observed_origin: None,
        observed_final: None,
        diagnostic_code: Some("test_failure".into()),
    }
}

fn ready() -> StageSurveyAttempt {
    StageSurveyAttempt {
        number: 0,
        outcome: StageSurveyAttemptOutcome::Ready,
        exit_code: Some(0),
        elapsed_millis: 40,
        observation_sha256: Some(digest(8)),
        actor_catalog_sha256: Some(digest(9)),
        observed_actor_count: Some(48),
        retained_actor_count: Some(48),
        actor_catalog_truncated: Some(false),
        state_sequence_sha256: Some(digest(10)),
        observed_origin: Some(StageSurveyObservedOrigin {
            stage: Some("F_SP103".into()),
            room: 0,
            point: 0,
            layer: 3,
            player_ready: true,
        }),
        observed_final: Some(StageSurveyObservedOrigin {
            stage: Some("F_SP103".into()),
            room: 0,
            point: 0,
            layer: 3,
            player_ready: true,
        }),
        diagnostic_code: None,
    }
}

#[test]
fn resume_schedules_only_unfinalized_candidates() {
    let catalog = catalog();
    let mut ledger = ledger(&catalog, 2);
    let first = catalog.candidates[0].id.clone();
    ledger
        .record_attempt(
            &catalog,
            &first,
            failed(StageSurveyAttemptOutcome::NativeReadinessTimeout),
        )
        .unwrap();
    assert_eq!(ledger.next_candidates(&catalog, 10).unwrap().len(), 2);
    ledger
        .record_attempt(
            &catalog,
            &first,
            failed(StageSurveyAttemptOutcome::NativeReadinessTimeout),
        )
        .unwrap();
    assert_eq!(
        ledger.cases[0].classification,
        Some(StageSurveyClassification::RepeatedReadinessTimeout)
    );
    assert_eq!(ledger.next_candidates(&catalog, 10).unwrap().len(), 1);
}

#[test]
fn ready_case_finalizes_immediately_and_round_trips_canonically() {
    let catalog = catalog();
    let mut ledger = ledger(&catalog, 3);
    ledger
        .record_attempt(&catalog, &catalog.candidates[0].id, ready())
        .unwrap();
    assert_eq!(
        ledger.progress(&catalog).unwrap(),
        StageSurveyProgress {
            total: 2,
            finalized: 1,
            pending: 1,
            attempted: 1,
            classifications: BTreeMap::from([("ready".into(), 1)]),
        }
    );
    let bytes = ledger.canonical_bytes(&catalog).unwrap();
    assert_eq!(
        StageSurveyLedger::decode_canonical(&bytes, &catalog).unwrap(),
        ledger
    );
    assert_ne!(ledger.digest(&catalog).unwrap(), Digest::ZERO);
}

#[test]
fn compaction_preserves_ledger_identity_and_is_repeatable() {
    let catalog = catalog();
    let mut ledger = ledger(&catalog, 1);
    let observation = vec![0x31; 192 * 1024];
    let actors = vec![0x72; 96 * 1024];
    let mut attempt = ready();
    attempt.observation_sha256 = Some(Digest(Sha256::digest(&observation).into()));
    attempt.actor_catalog_sha256 = Some(Digest(Sha256::digest(&actors).into()));
    ledger
        .record_attempt(&catalog, &catalog.candidates[0].id, attempt)
        .unwrap();

    let root = temporary_root("artifact-compaction");
    let attempt_number = ledger.cases[0].attempts[0].number;
    let artifact_root = root
        .join("cases")
        .join(stage_survey_case_storage_id(&catalog.candidates[0].id).to_string())
        .join(format!("attempt-{attempt_number:03}-run-00000"));
    fs::create_dir_all(&artifact_root).unwrap();
    let observation_path = artifact_root.join("observation.trace");
    let actor_path = artifact_root.join("actors.json");
    fs::write(&observation_path, &observation).unwrap();
    fs::write(&actor_path, &actors).unwrap();

    let first = compact_stage_survey_artifacts(&catalog, &ledger, &root).unwrap();
    assert_eq!(
        first.schema,
        "dusklight-stage-survey-artifact-compaction/v1"
    );
    assert_eq!(first.ledger_sha256, ledger.digest(&catalog).unwrap());
    assert_eq!(first.ready_cases, 1);
    assert_eq!(first.verified_artifacts, 2);
    assert_eq!(first.compacted_artifacts, 2);
    assert_eq!(
        first.logical_raw_bytes,
        u64::try_from(observation.len() + actors.len()).unwrap()
    );
    assert!(first.stored_bytes < first.logical_raw_bytes);
    assert_eq!(
        first.storage_savings_bytes,
        first.logical_raw_bytes - first.stored_bytes
    );
    assert!(!observation_path.exists());
    assert!(!actor_path.exists());

    let second = compact_stage_survey_artifacts(&catalog, &ledger, &root).unwrap();
    assert_eq!(second.compacted_artifacts, 0);
    assert_eq!(second.verified_artifacts, first.verified_artifacts);
    assert_eq!(second.logical_raw_bytes, first.logical_raw_bytes);
    assert_eq!(second.stored_bytes, first.stored_bytes);
    assert_eq!(second.storage_savings_bytes, first.storage_savings_bytes);
    assert_eq!(
        read_survey_artifact(
            &observation_path,
            ledger.cases[0].attempts[0].observation_sha256.unwrap()
        )
        .unwrap(),
        Some(observation)
    );
    assert_eq!(
        read_survey_artifact(
            &actor_path,
            ledger.cases[0].attempts[0].actor_catalog_sha256.unwrap()
        )
        .unwrap(),
        Some(actors)
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn identity_mismatch_and_attempt_after_finalization_fail_closed() {
    let catalog = catalog();
    let mut ledger = ledger(&catalog, 2);
    let candidate = catalog.candidates[0].id.clone();
    ledger
        .record_attempt(&catalog, &candidate, ready())
        .unwrap();
    assert!(
        ledger
            .record_attempt(
                &catalog,
                &candidate,
                failed(StageSurveyAttemptOutcome::ProcessCrash)
            )
            .is_err()
    );
    let mut other = catalog.clone();
    other.candidates[0].point = 99;
    other.candidates[0].id = "F_SP103/room/0/point/99/layer/-1".into();
    assert!(ledger.validate(&other).is_err());
}

#[test]
fn neutral_probe_owns_only_port_zero_and_preserves_exact_boot_origin() {
    let candidate = &catalog().candidates[0];
    let tape = survey_probe_tape(
        candidate,
        &StageSurveyPolicy {
            probe_ticks: 30,
            probe: StageSurveyProbeKind::Neutral,
            native_stage_readiness_ticks: DEFAULT_NATIVE_STAGE_READINESS_TICKS,
            host_timeout_millis: 120_000,
            maximum_attempts_per_case: 1,
            fidelity_profile: STAGE_SURVEY_FIDELITY.into(),
        },
    )
    .unwrap();
    assert_eq!(
        tape.boot,
        TapeBoot::Stage {
            stage: "F_SP103".into(),
            room: 0,
            point: 0,
            layer: -1,
            save_slot: None,
            fixture: None,
        }
    );
    assert_eq!(tape.frames.len(), 30);
    assert!(
        tape.frames
            .iter()
            .all(|frame| { frame.owned_ports == 1 && frame.pads == [RawPadState::default(); 4] })
    );
}

#[test]
fn generic_probe_profiles_change_only_the_declared_pad_factor() {
    let candidate = &catalog().candidates[0];
    let build = |probe| {
        survey_probe_tape(
            candidate,
            &StageSurveyPolicy {
                probe_ticks: 20,
                probe,
                native_stage_readiness_ticks: DEFAULT_NATIVE_STAGE_READINESS_TICKS,
                host_timeout_millis: 120_000,
                maximum_attempts_per_case: 1,
                fidelity_profile: STAGE_SURVEY_FIDELITY.into(),
            },
        )
        .unwrap()
    };

    let movement = build(StageSurveyProbeKind::Movement);
    assert!(movement.frames[..5].iter().all(neutral_frame));
    assert!(movement.frames[15..].iter().all(neutral_frame));
    assert!(movement.frames[5..15].iter().all(|frame| {
        frame.owned_ports == 1
            && frame.pads[0].stick_y == 100
            && frame.pads[0].stick_x == 0
            && frame.pads[0].buttons == 0
    }));

    let camera = build(StageSurveyProbeKind::Camera);
    assert!(camera.frames[5..15].iter().all(|frame| {
        frame.pads[0].substick_x == 80
            && frame.pads[0].substick_y == 0
            && frame.pads[0].buttons == 0
    }));

    let targeting = build(StageSurveyProbeKind::Targeting);
    assert!(
        targeting.frames[5..15]
            .iter()
            .all(|frame| frame.pads[0].buttons == BUTTON_L)
    );

    let actions = build(StageSurveyProbeKind::BasicActions);
    let presses = actions
        .frames
        .iter()
        .filter_map(|frame| (frame.pads[0].buttons != 0).then_some(frame.pads[0].buttons))
        .collect::<Vec<_>>();
    assert_eq!(presses, [BUTTON_A, BUTTON_B, BUTTON_X, BUTTON_Y]);
    assert!(actions.frames.iter().all(|frame| {
        frame.owned_ports == 1
            && frame.pads[0].stick_x == 0
            && frame.pads[0].stick_y == 0
            && frame.pads[0].substick_x == 0
            && frame.pads[0].substick_y == 0
    }));
}

#[test]
fn contact_sweep_covers_eight_directions_with_neutral_release_phases() {
    let tape = survey_probe_tape(
        &catalog().candidates[0],
        &StageSurveyPolicy {
            probe_ticks: 80,
            probe: StageSurveyProbeKind::ContactSweep,
            native_stage_readiness_ticks: DEFAULT_NATIVE_STAGE_READINESS_TICKS,
            host_timeout_millis: 120_000,
            maximum_attempts_per_case: 1,
            fidelity_profile: STAGE_SURVEY_FIDELITY.into(),
        },
    )
    .unwrap();
    assert!(tape.frames[..20].iter().all(neutral_frame));
    assert!(tape.frames[60..].iter().all(neutral_frame));
    let active = &tape.frames[20..60];
    for direction in [
        (0, 100),
        (71, 71),
        (100, 0),
        (71, -71),
        (0, -100),
        (-71, -71),
        (-100, 0),
        (-71, 71),
    ] {
        assert!(
            active
                .iter()
                .any(|frame| { (frame.pads[0].stick_x, frame.pads[0].stick_y) == direction })
        );
    }
    assert!(active.iter().any(neutral_frame));
    assert!(tape.frames.iter().all(|frame| {
        frame.owned_ports == 1
            && frame.pads[0].buttons == 0
            && frame.pads[0].substick_x == 0
            && frame.pads[0].substick_y == 0
            && frame.pads[0].trigger_left == 0
            && frame.pads[0].trigger_right == 0
    }));
}

#[test]
fn actor_activation_sweep_crosses_ordinary_pad_factors_without_selecting_an_actor() {
    let tape = survey_probe_tape(
        &catalog().candidates[0],
        &StageSurveyPolicy {
            probe_ticks: StageSurveyProbeKind::ActorActivation.minimum_ticks(),
            probe: StageSurveyProbeKind::ActorActivation,
            native_stage_readiness_ticks: DEFAULT_NATIVE_STAGE_READINESS_TICKS,
            host_timeout_millis: 120_000,
            maximum_attempts_per_case: 1,
            fidelity_profile: STAGE_SURVEY_FIDELITY.into(),
        },
    )
    .unwrap();
    assert!(tape.frames[..90].iter().all(neutral_frame));
    assert!(tape.frames[270..].iter().all(neutral_frame));
    let active = &tape.frames[90..270];
    for button in ACTIVATION_BUTTONS {
        assert!(active.iter().any(|frame| {
            frame.pads[0].buttons == button
                && frame.pads[0].stick_x == 0
                && frame.pads[0].stick_y == 0
        }));
        for (stick_x, stick_y) in STICK_DIRECTIONS {
            assert!(active.iter().any(|frame| {
                frame.pads[0].buttons == button
                    && frame.pads[0].stick_x == stick_x
                    && frame.pads[0].stick_y == stick_y
            }));
        }
    }
    assert!(active.iter().any(neutral_frame));
    assert!(tape.frames.iter().all(|frame| {
        frame.owned_ports == 1
            && frame.pads[1..] == [RawPadState::default(); 3]
            && frame.pads[0].buttons.count_ones() <= 1
            && frame.pads[0].substick_x == 0
            && frame.pads[0].substick_y == 0
            && frame.pads[0].trigger_left == 0
            && frame.pads[0].trigger_right == 0
    }));
}

#[test]
fn loading_sweep_uses_only_sustained_directional_motion_and_release() {
    let tape = survey_probe_tape(
        &catalog().candidates[0],
        &StageSurveyPolicy {
            probe_ticks: StageSurveyProbeKind::LoadingSweep.minimum_ticks(),
            probe: StageSurveyProbeKind::LoadingSweep,
            native_stage_readiness_ticks: DEFAULT_NATIVE_STAGE_READINESS_TICKS,
            host_timeout_millis: 120_000,
            maximum_attempts_per_case: 1,
            fidelity_profile: STAGE_SURVEY_FIDELITY.into(),
        },
    )
    .unwrap();
    assert!(tape.frames[..180].iter().all(neutral_frame));
    assert!(tape.frames[540..].iter().all(neutral_frame));
    let active = &tape.frames[180..540];
    for direction in STICK_DIRECTIONS {
        assert!(
            active
                .iter()
                .filter(|frame| { (frame.pads[0].stick_x, frame.pads[0].stick_y) == direction })
                .count()
                >= 22
        );
    }
    assert!(active.iter().any(neutral_frame));
    assert!(tape.frames.iter().all(|frame| {
        frame.owned_ports == 1
            && frame.pads[1..] == [RawPadState::default(); 3]
            && frame.pads[0].buttons == 0
            && frame.pads[0].substick_x == 0
            && frame.pads[0].substick_y == 0
            && frame.pads[0].trigger_left == 0
            && frame.pads[0].trigger_right == 0
    }));
}

#[test]
fn neutral_policy_preserves_legacy_canonical_shape_and_probe_minima_fail_closed() {
    let catalog = catalog();
    let neutral = ledger(&catalog, 1);
    let bytes = neutral.canonical_bytes(&catalog).unwrap();
    let text = std::str::from_utf8(&bytes).unwrap();
    assert!(!text.contains("\"probe\":"));
    assert!(!text.contains("\"native_stage_readiness_ticks\":"));
    assert_eq!(
        StageSurveyLedger::decode_canonical(&bytes, &catalog).unwrap(),
        neutral
    );

    let mut bounded_readiness = neutral.clone();
    bounded_readiness.policy.native_stage_readiness_ticks = 300;
    let bytes = bounded_readiness.canonical_bytes(&catalog).unwrap();
    assert!(
        std::str::from_utf8(&bytes)
            .unwrap()
            .contains("\"native_stage_readiness_ticks\":300")
    );
    assert_eq!(
        StageSurveyLedger::decode_canonical(&bytes, &catalog).unwrap(),
        bounded_readiness
    );
    bounded_readiness.policy.native_stage_readiness_ticks = 0;
    assert!(bounded_readiness.validate(&catalog).is_err());

    let mut movement = neutral.clone();
    movement.policy.probe = StageSurveyProbeKind::Movement;
    movement.policy.probe_ticks = 2;
    assert!(movement.validate(&catalog).is_err());
    assert!(survey_probe_tape(&catalog.candidates[0], &movement.policy).is_err());

    movement.policy.probe_ticks = 4;
    let bytes = movement.canonical_bytes(&catalog).unwrap();
    assert!(
        std::str::from_utf8(&bytes)
            .unwrap()
            .contains("\"probe\":\"movement\"")
    );

    movement.policy.probe = StageSurveyProbeKind::ContactSweep;
    movement.policy.probe_ticks = 79;
    assert!(movement.validate(&catalog).is_err());
    movement.policy.probe_ticks = 80;
    assert!(movement.validate(&catalog).is_ok());

    movement.policy.probe = StageSurveyProbeKind::ActorActivation;
    movement.policy.probe_ticks = 359;
    assert!(movement.validate(&catalog).is_err());
    movement.policy.probe_ticks = 360;
    assert!(movement.validate(&catalog).is_ok());

    movement.policy.probe = StageSurveyProbeKind::LoadingSweep;
    movement.policy.probe_ticks = 719;
    assert!(movement.validate(&catalog).is_err());
    movement.policy.probe_ticks = 720;
    assert!(movement.validate(&catalog).is_ok());
}

#[test]
fn probe_acceptance_requires_exact_consumed_pad_on_every_owned_port() {
    let mut frame = InputFrame {
        owned_ports: 1,
        ..InputFrame::default()
    };
    frame.pads[0].stick_y = 100;
    let mut applied = TraceAppliedPads {
        valid_ports: 1,
        owned_ports: 1,
        pads: frame.pads,
    };
    assert!(applied_pad_matches_frame(Some(&applied), &frame));

    applied.pads[0].stick_y = 99;
    assert!(!applied_pad_matches_frame(Some(&applied), &frame));
    applied.pads[0] = frame.pads[0];
    applied.valid_ports = 0;
    assert!(!applied_pad_matches_frame(Some(&applied), &frame));
    applied.valid_ports = 1;
    applied.owned_ports = 3;
    assert!(!applied_pad_matches_frame(Some(&applied), &frame));
    assert!(!applied_pad_matches_frame(None, &frame));

    applied.owned_ports = 1;
    applied.pads[1].buttons = BUTTON_A;
    assert!(applied_pad_matches_frame(Some(&applied), &frame));
}

fn neutral_frame(frame: &InputFrame) -> bool {
    frame.owned_ports == 1 && frame.pads == [RawPadState::default(); 4]
}
