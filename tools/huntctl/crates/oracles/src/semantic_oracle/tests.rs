use super::*;
use crate::tape::TapeBoot;
use crate::trace::{
    TraceAnimationLane, TraceCollisionWall, TracePhase, TracePlayerAction,
    TracePlayerBackgroundCollision,
};

fn collision(flags: u32) -> TracePlayerBackgroundCollision {
    TracePlayerBackgroundCollision {
        flags,
        ground_height: 0.0,
        roof_height: 0.0,
        water_height: 0.0,
        ground_bg_index: None,
        ground_poly_index: None,
        ground_owner_session_process_id: None,
        ground_plane: [0.0; 4],
        ground_identity_present: false,
        roof_bg_index: None,
        roof_poly_index: None,
        roof_owner_session_process_id: None,
        roof_identity_present: false,
        water_bg_index: None,
        water_poly_index: None,
        water_owner_session_process_id: None,
        water_identity_present: false,
        walls: std::array::from_fn(|_| TraceCollisionWall {
            identity_present: false,
            bg_index: None,
            poly_index: None,
            owner_session_process_id: None,
            angle_y: 0,
            flags: 0,
        }),
        old_position: [0.0; 3],
        resolved_frame_displacement: [0.0; 3],
        final_position: [0.0; 3],
        solver: None,
    }
}

fn trace(exhausted: bool) -> DecodedTrace {
    let mut records = Vec::new();
    for tick in 1..=2 {
        let mut record = TraceRecord {
            simulation_tick: tick,
            boundary_index: tick + 1,
            tape_frame: Some(tick - 1),
            observation_phase: TracePhase::PostSimulation,
            stage_name: "F_SP103".into(),
            room: 1,
            position: [tick as f32, 0.0, 0.0],
            event_id: 9,
            ..TraceRecord::default()
        };
        for channel in [
            TraceChannel::Stage,
            TraceChannel::PlayerMotion,
            TraceChannel::PlayerAction,
            TraceChannel::Event,
        ] {
            record
                .channel_status
                .insert(channel, TraceChannelStatus::Present);
        }
        record.player_action = Some(TracePlayerAction {
            procedure_id: 7,
            mode_flags: 4,
            procedure_context_raw: [0; 6],
            damage_wait_timer: 0,
            sword_at_up_time: 0,
            ice_damage_wait_timer: 0,
            sword_change_wait_timer: 0,
            under_animations: std::array::from_fn(|_| TraceAnimationLane {
                resource_id: if tick == 2 { 42 } else { 1 },
                frame: 3.0,
                rate: 1.0,
            }),
            upper_animations: std::array::from_fn(|_| TraceAnimationLane {
                resource_id: 2,
                frame: 0.0,
                rate: 1.0,
            }),
            do_status: 0,
            talk_partner: None,
            grabbed_actor: None,
        });
        records.push(record);
    }
    DecodedTrace {
        version: 2,
        boot: TapeBoot::Process,
        tick_rate_numerator: 30,
        tick_rate_denominator: 1,
        requested_channels: 0,
        capacity_exhausted: exhausted,
        retention: None,
        channel_formats: BTreeMap::new(),
        records,
    }
}

#[test]
fn reached_and_avoided_cover_trace_and_supplemental_domains() {
    let targets = vec![
        (
            "stage",
            OraclePolarity::Reached,
            OracleTarget::Stage {
                stage: "F_SP103".into(),
            },
        ),
        (
            "region",
            OraclePolarity::Reached,
            OracleTarget::Region {
                stage: Some("F_SP103".into()),
                room: Some(1),
                min: [2.0, 0.0, 0.0],
                max: [2.0, 0.0, 0.0],
            },
        ),
        (
            "action",
            OraclePolarity::Reached,
            OracleTarget::Action {
                procedure_id: 7,
                mode_all: 4,
                mode_none: 2,
            },
        ),
        (
            "animation",
            OraclePolarity::Reached,
            OracleTarget::Animation {
                bank: AnimationBank::Under,
                lane: None,
                resource_id: 42,
                frame_min: Some(3.0),
                frame_max: Some(3.0),
            },
        ),
        (
            "event",
            OraclePolarity::Reached,
            OracleTarget::Event {
                id: Some(9),
                name_hash: None,
                mode: None,
                status: None,
            },
        ),
        (
            "flag",
            OraclePolarity::Reached,
            OracleTarget::Flag {
                domain: FlagDomain::Event,
                room: None,
                index: 5,
                value: true,
            },
        ),
        (
            "actor",
            OraclePolarity::Reached,
            OracleTarget::ActorState {
                stage: "F_SP103".into(),
                home_room: 1,
                set_id: 2,
                actor_name: 3,
                current_room: Some(1),
                health: Some(4),
                status_all: 1,
                status_none: 2,
            },
        ),
        (
            "avoided",
            OraclePolarity::Avoided,
            OracleTarget::Room {
                stage: "F_SP103".into(),
                room: 9,
            },
        ),
    ];
    let program = SemanticOracleProgram {
        schema: SEMANTIC_ORACLE_SCHEMA_V1.into(),
        oracles: targets
            .into_iter()
            .map(|(name, polarity, target)| SemanticOracle {
                name: name.into(),
                polarity,
                target,
            })
            .collect(),
    };
    let snapshots = (1..=2)
        .map(|tick| SupplementalSnapshot {
            simulation_tick: tick,
            flags: vec![FlagObservation {
                domain: FlagDomain::Event,
                room: None,
                index: 5,
                value: true,
            }],
            actors: vec![ActorObservation {
                stage: "F_SP103".into(),
                home_room: 1,
                set_id: 2,
                actor_name: 3,
                current_room: 1,
                health: 4,
                status: 1,
            }],
        })
        .collect();
    let report = program
        .evaluate(
            &trace(false),
            &SupplementalObservations {
                snapshots,
                flags_complete: true,
                actors_complete: true,
                run_outcome: None,
            },
        )
        .unwrap();
    assert!(
        report
            .results
            .iter()
            .all(|result| result.disposition == OracleDisposition::Satisfied)
    );
    assert_eq!(
        report.results[3]
            .first_match
            .as_ref()
            .unwrap()
            .simulation_tick,
        2
    );
}

#[test]
fn avoidance_is_indeterminate_when_trace_or_supplemental_coverage_is_incomplete() {
    let program = SemanticOracleProgram {
        schema: SEMANTIC_ORACLE_SCHEMA_V1.into(),
        oracles: vec![
            SemanticOracle {
                name: "avoid-room".into(),
                polarity: OraclePolarity::Avoided,
                target: OracleTarget::Room {
                    stage: "F_SP103".into(),
                    room: 9,
                },
            },
            SemanticOracle {
                name: "avoid-flag".into(),
                polarity: OraclePolarity::Avoided,
                target: OracleTarget::Flag {
                    domain: FlagDomain::Event,
                    room: None,
                    index: 5,
                    value: true,
                },
            },
        ],
    };
    let report = program
        .evaluate(&trace(true), &SupplementalObservations::default())
        .unwrap();
    assert!(
        report
            .results
            .iter()
            .all(|result| result.disposition == OracleDisposition::Indeterminate)
    );
}

#[test]
fn checked_in_semantic_oracle_catalog_is_valid() {
    let program: SemanticOracleProgram = serde_json::from_str(include_str!(
        "../../../../../../tests/fixtures/automation/semantic_oracles.json"
    ))
    .unwrap();
    program.validate().unwrap();
}

#[test]
fn checked_in_run_outcome_fixture_is_valid() {
    let outcome: RunOutcomeEvidence = serde_json::from_str(include_str!(
        "../../../../../../tests/fixtures/automation/run_outcome.json"
    ))
    .unwrap();
    validate_run_outcome(&outcome).unwrap();
}

#[test]
fn collision_load_motion_and_invalid_state_oracles_retain_exact_evidence() {
    let mut source = trace(false);
    source.records[0].position = [-2.0, 0.0, 0.0];
    source.records[1].position = [2.0, -20.0, 0.0];
    source.records[1].velocity = [100.0, 0.0, 0.0];
    source.records[1].next_stage_enabled = true;
    source.records[1].next_stage_name = "F_WRONG".into();
    source.records[1].next_room = 3;
    source.records.push(source.records[1].clone());
    source.records[2].simulation_tick = 3;
    source.records[2].boundary_index = 4;
    source.records[2].tape_frame = Some(2);
    source.records[2].position = [5000.0, -20.0, 0.0];
    source.records.push(source.records[2].clone());
    source.records[3].simulation_tick = 4;
    source.records[3].boundary_index = 5;
    source.records[3].tape_frame = Some(3);
    source.records[3].stage_name = "F_WRONG".into();
    source.records[3].room = 3;
    for record in &mut source.records {
        record.channel_status.insert(
            TraceChannel::PlayerBackgroundCollision,
            TraceChannelStatus::Present,
        );
        record.player_background_collision = Some(collision(0));
    }
    let location = |stage: &str, room| LocationTarget {
        stage: stage.into(),
        room,
        layer: None,
        point: None,
    };
    let targets = vec![
        OracleTarget::CollisionCrossing {
            point: [0.0; 3],
            normal: [1.0, 0.0, 0.0],
            tolerance: 0.1,
            contact_mask: 2,
        },
        OracleTarget::OutOfBounds {
            allowed_min: [-1000.0; 3],
            allowed_max: [1000.0; 3],
        },
        OracleTarget::VoidSurvival {
            below_y: -10.0,
            minimum_ticks: 2,
        },
        OracleTarget::UnexpectedLoad {
            allowed_destinations: vec![location("F_EXPECT", 1)],
        },
        OracleTarget::WrongWarp {
            expected: location("F_EXPECT", 1),
        },
        OracleTarget::ExcessiveMotion {
            max_displacement: Some(100.0),
            max_speed: Some(50.0),
        },
        OracleTarget::ImpossibleCoordinates { max_abs: 4096.0 },
    ];
    let program = SemanticOracleProgram {
        schema: SEMANTIC_ORACLE_SCHEMA_V1.into(),
        oracles: targets
            .into_iter()
            .enumerate()
            .map(|(index, target)| SemanticOracle {
                name: format!("safety-{index}"),
                polarity: OraclePolarity::Reached,
                target,
            })
            .collect(),
    };
    let report = program
        .evaluate(&source, &SupplementalObservations::default())
        .unwrap();
    assert!(
        report
            .results
            .iter()
            .all(|result| result.disposition == OracleDisposition::Satisfied)
    );
    assert_eq!(
        report.results[0]
            .first_match
            .as_ref()
            .unwrap()
            .simulation_tick,
        2
    );
    assert_eq!(
        report.results[2]
            .first_match
            .as_ref()
            .unwrap()
            .simulation_tick,
        3
    );

    source.records[0].position[0] = f32::NAN;
    let nonfinite = SemanticOracleProgram {
        schema: SEMANTIC_ORACLE_SCHEMA_V1.into(),
        oracles: vec![SemanticOracle {
            name: "nan".into(),
            polarity: OraclePolarity::Reached,
            target: OracleTarget::NonFiniteState,
        }],
    }
    .evaluate(&source, &SupplementalObservations::default())
    .unwrap();
    assert_eq!(
        nonfinite.results[0].disposition,
        OracleDisposition::Satisfied
    );
}

#[test]
fn run_outcome_oracles_retain_typed_failure_evidence() {
    let targets = vec![
        OracleTarget::ActorCorruption {
            actor_name: Some(77),
            field: Some("health".into()),
        },
        OracleTarget::SlotExhaustion,
        OracleTarget::WatchedFieldCorruption {
            field: Some("player.inventory.wallet".into()),
        },
        OracleTarget::HeapFailure {
            heap: Some("game".into()),
        },
        OracleTarget::Crash,
        OracleTarget::Softlock { minimum_ticks: 10 },
        OracleTarget::ControlLoss { minimum_ticks: 5 },
    ];
    let program = SemanticOracleProgram {
        schema: SEMANTIC_ORACLE_SCHEMA_V1.into(),
        oracles: targets
            .into_iter()
            .enumerate()
            .map(|(index, target)| SemanticOracle {
                name: format!("run-failure-{index}"),
                polarity: OraclePolarity::Reached,
                target,
            })
            .collect(),
    };
    let outcome = RunOutcomeEvidence {
        schema: RUN_OUTCOME_SCHEMA_V1.into(),
        monitored: vec![
            RunEvidenceKind::ActorIntegrity,
            RunEvidenceKind::ActorSlots,
            RunEvidenceKind::WatchedFields,
            RunEvidenceKind::Heap,
            RunEvidenceKind::Progress,
            RunEvidenceKind::Control,
        ],
        termination: Some(RunTermination::Crashed {
            exit_code: None,
            signal: Some(11),
            reason: "segmentation fault".into(),
        }),
        anomalies: vec![
            RunAnomalyObservation::ActorCorruption {
                simulation_tick: 10,
                tape_frame: Some(9),
                actor: RunActorIdentity {
                    process_id: Some(19),
                    actor_name: 77,
                    stage: Some("F_SP103".into()),
                    home_room: Some(1),
                    set_id: Some(4),
                },
                field: "health".into(),
                expected: "4".into(),
                actual: "-2147483648".into(),
            },
            RunAnomalyObservation::SlotExhaustion {
                simulation_tick: 11,
                tape_frame: Some(10),
                active_slots: 256,
                capacity: 256,
                requested_actor_name: Some(80),
            },
            RunAnomalyObservation::WatchedFieldCorruption {
                simulation_tick: 12,
                tape_frame: Some(11),
                field: "player.inventory.wallet".into(),
                expected: "0..=999".into(),
                actual: "65535".into(),
            },
            RunAnomalyObservation::HeapFailure {
                simulation_tick: Some(13),
                tape_frame: Some(12),
                heap: "game".into(),
                operation: "alloc".into(),
                requested_bytes: 4096,
                free_bytes: 1024,
            },
            RunAnomalyObservation::Softlock {
                start_tick: 20,
                end_tick: 29,
                tape_frame: Some(28),
                last_progress: "event 4 phase 2".into(),
                reason: "simulation advanced without semantic progress".into(),
            },
            RunAnomalyObservation::ControlLoss {
                start_tick: 30,
                end_tick: 34,
                tape_frame: Some(33),
                procedure_id: Some(7),
                reason: "input ownership stayed disabled".into(),
            },
        ],
    };
    let report = program
        .evaluate(
            &trace(true),
            &SupplementalObservations {
                run_outcome: Some(outcome),
                ..SupplementalObservations::default()
            },
        )
        .unwrap();
    assert!(
        report
            .results
            .iter()
            .all(|result| result.disposition == OracleDisposition::Satisfied)
    );
    assert!(matches!(
        report.results[0].first_match.as_ref().unwrap().facts,
        OracleFacts::ActorCorruption { .. }
    ));
    assert!(matches!(
        report.results[4].first_match.as_ref().unwrap().facts,
        OracleFacts::Crash {
            signal: Some(11),
            ..
        }
    ));
}

#[test]
fn hang_and_avoided_run_failures_require_declared_coverage() {
    let reached_hang = SemanticOracleProgram {
        schema: SEMANTIC_ORACLE_SCHEMA_V1.into(),
        oracles: vec![SemanticOracle {
            name: "hung".into(),
            polarity: OraclePolarity::Reached,
            target: OracleTarget::Hang {
                minimum_stalled_millis: 2_000,
            },
        }],
    };
    let timeout = RunOutcomeEvidence {
        schema: RUN_OUTCOME_SCHEMA_V1.into(),
        monitored: vec![RunEvidenceKind::Progress],
        termination: Some(RunTermination::TimedOut {
            wall_time_millis: 10_000,
            stalled_millis: 3_000,
            last_simulation_tick: 99,
        }),
        anomalies: vec![],
    };
    let report = reached_hang
        .evaluate(
            &trace(true),
            &SupplementalObservations {
                run_outcome: Some(timeout),
                ..SupplementalObservations::default()
            },
        )
        .unwrap();
    assert_eq!(report.results[0].disposition, OracleDisposition::Satisfied);
    assert_eq!(
        report.results[0]
            .first_match
            .as_ref()
            .unwrap()
            .simulation_tick,
        99
    );

    let avoided = SemanticOracleProgram {
        schema: SEMANTIC_ORACLE_SCHEMA_V1.into(),
        oracles: vec![
            SemanticOracle {
                name: "no-crash".into(),
                polarity: OraclePolarity::Avoided,
                target: OracleTarget::Crash,
            },
            SemanticOracle {
                name: "no-control-loss".into(),
                polarity: OraclePolarity::Avoided,
                target: OracleTarget::ControlLoss { minimum_ticks: 5 },
            },
        ],
    };
    let missing = avoided
        .evaluate(&trace(false), &SupplementalObservations::default())
        .unwrap();
    assert!(
        missing
            .results
            .iter()
            .all(|result| result.disposition == OracleDisposition::Indeterminate)
    );
    let clean = RunOutcomeEvidence {
        schema: RUN_OUTCOME_SCHEMA_V1.into(),
        monitored: vec![RunEvidenceKind::Control],
        termination: Some(RunTermination::Completed { exit_code: 0 }),
        anomalies: vec![],
    };
    let report = avoided
        .evaluate(
            &trace(true),
            &SupplementalObservations {
                run_outcome: Some(clean),
                ..SupplementalObservations::default()
            },
        )
        .unwrap();
    assert!(
        report
            .results
            .iter()
            .all(|result| result.disposition == OracleDisposition::Satisfied)
    );
}

#[test]
fn progression_and_save_oracles_retain_semantic_source_facts() {
    let targets = vec![
        OracleTarget::DuplicateItemReward {
            grant_kind: Some(GrantKind::Reward),
            id: Some(42),
        },
        OracleTarget::PreservedStorageState {
            field: Some("carry.actor".into()),
        },
        OracleTarget::EventQueueing {
            event_id: Some(5),
            minimum_depth: 2,
        },
        OracleTarget::SequenceBreak {
            sequence: Some("forest-entry".into()),
        },
        OracleTarget::SaveStateAnomaly {
            slot: Some(1),
            field: Some("event_flags.42".into()),
        },
    ];
    let program = SemanticOracleProgram {
        schema: SEMANTIC_ORACLE_SCHEMA_V1.into(),
        oracles: targets
            .into_iter()
            .enumerate()
            .map(|(index, target)| SemanticOracle {
                name: format!("progression-{index}"),
                polarity: OraclePolarity::Reached,
                target,
            })
            .collect(),
    };
    let outcome = RunOutcomeEvidence {
        schema: RUN_OUTCOME_SCHEMA_V1.into(),
        monitored: vec![
            RunEvidenceKind::InventoryRewards,
            RunEvidenceKind::Storage,
            RunEvidenceKind::EventQueue,
            RunEvidenceKind::Sequence,
            RunEvidenceKind::SaveState,
        ],
        termination: Some(RunTermination::Completed { exit_code: 0 }),
        anomalies: vec![
            RunAnomalyObservation::DuplicateItemReward {
                simulation_tick: 50,
                tape_frame: Some(49),
                grant_kind: GrantKind::Reward,
                id: 42,
                first_source: "chest actor 100".into(),
                duplicate_source: "event reward 9".into(),
                total_grants: 2,
            },
            RunAnomalyObservation::PreservedStorageState {
                simulation_tick: 51,
                tape_frame: Some(50),
                field: "carry.actor".into(),
                expected_reset: "none".into(),
                actual: "placed:F_SP103:1:17".into(),
            },
            RunAnomalyObservation::EventQueueing {
                simulation_tick: 52,
                tape_frame: Some(51),
                running_event_id: Some(4),
                queued_event_ids: vec![5, 9],
            },
            RunAnomalyObservation::SequenceBreak {
                simulation_tick: 53,
                tape_frame: Some(52),
                sequence: "forest-entry".into(),
                expected_step: "talk-to-ordona".into(),
                actual_step: "enter-faron".into(),
            },
            RunAnomalyObservation::SaveStateAnomaly {
                simulation_tick: Some(54),
                tape_frame: Some(53),
                slot: 1,
                field: "event_flags.42".into(),
                expected: "false".into(),
                actual: "true".into(),
            },
        ],
    };
    let report = program
        .evaluate(
            &trace(true),
            &SupplementalObservations {
                run_outcome: Some(outcome),
                ..SupplementalObservations::default()
            },
        )
        .unwrap();
    assert!(
        report
            .results
            .iter()
            .all(|result| result.disposition == OracleDisposition::Satisfied)
    );
    assert!(matches!(
        report.results[4].first_match.as_ref().unwrap().facts,
        OracleFacts::SaveStateAnomaly {
            slot: 1,
            ref field,
            ..
        } if field == "event_flags.42"
    ));
}
