
use super::*;
use crate::residual_action::{
    AnalogChannel, AnalogResidual, ResidualCandidate, TemporalBasis, compile_residual_candidate,
};
use dusklight_automation_contracts::tape::InputFrame;

fn digest(byte: u8) -> Digest {
    Digest([byte; 32])
}

fn config(parent_bytes: &[u8], capacity: u64) -> ResidualRetentionConfig {
    ResidualRetentionConfig {
        parent_tape_sha256: sha256(parent_bytes),
        terminal_program_sha256: digest(1),
        terminal_definition_sha256: digest(2),
        exploration_horizon_ticks: 160,
        promotion_before_tick: 125,
        maximum_candidates: 64,
        failures: FailureRetentionPolicy::DiversityReservoir { capacity },
    }
}

fn parent() -> (InputTape, Vec<u8>) {
    let tape = InputTape {
        frames: (0..32)
            .map(|_| InputFrame {
                owned_ports: 1,
                ..InputFrame::default()
            })
            .collect(),
        ..InputTape::default()
    };
    let bytes = tape.encode().unwrap();
    (tape, bytes)
}

fn compiled(parent: &InputTape, bytes: &[u8], frame: u64, delta: i16) -> CompiledResidualCandidate {
    let candidate = ResidualCandidate::seal(
        bytes,
        vec![AnalogResidual {
            port: 0,
            channel: AnalogChannel::MainX,
            basis: TemporalBasis::ExactFrame { frame, delta },
        }],
        vec![],
    )
    .unwrap();
    compile_residual_candidate(parent, bytes, &candidate).unwrap()
}

fn evidence(
    compiled: &CompiledResidualCandidate,
    byte: u8,
    behavior: u8,
    verdict: ExactTerminalVerdict,
    shaped: i64,
) -> ResidualEvaluationEvidence {
    ResidualEvaluationEvidence {
        candidate_sha256: compiled.report.candidate_sha256,
        realized_tape_sha256: compiled.report.realized_tape_sha256,
        terminal_program_sha256: digest(1),
        terminal_definition_sha256: digest(2),
        evaluation_sha256: digest(byte),
        episode_sha256: digest(byte.wrapping_add(64)),
        behavior_sha256: digest(behavior),
        verdict,
        shaped_progress_millionths: Some(shaped),
        native_risk_events: Some(0),
    }
}

#[test]
fn retains_every_horizon_success_and_ranks_without_shaped_reward() {
    let (parent, bytes) = parent();
    let mut archive = ResidualOutcomeArchive::new(config(&bytes, 8)).unwrap();
    let slower = compiled(&parent, &bytes, 4, 5);
    archive
        .record(
            &slower,
            evidence(
                &slower,
                10,
                20,
                ExactTerminalVerdict::Reached {
                    first_hit_tick: 150,
                },
                i64::MAX,
            ),
        )
        .unwrap();
    let faster = compiled(&parent, &bytes, 8, 6);
    archive
        .record(
            &faster,
            evidence(
                &faster,
                11,
                21,
                ExactTerminalVerdict::Reached {
                    first_hit_tick: 124,
                },
                i64::MIN,
            ),
        )
        .unwrap();
    assert_eq!(archive.successes().len(), 2);
    assert_eq!(archive.successes()[0].first_hit_tick, 124);
    assert_eq!(archive.successes()[1].first_hit_tick, 150);
}

#[test]
fn generation_rank_keeps_every_pending_failure_outside_the_reservoir() {
    let (parent, bytes) = parent();
    let slow = compiled(&parent, &bytes, 2, 3);
    let miss_a = compiled(&parent, &bytes, 3, 4);
    let fast = compiled(&parent, &bytes, 4, 5);
    let miss_b = compiled(&parent, &bytes, 5, 6);
    let slow_evidence = evidence(
        &slow,
        40,
        9,
        ExactTerminalVerdict::Reached {
            first_hit_tick: 150,
        },
        i64::MAX,
    );
    let miss_a_evidence = evidence(&miss_a, 41, 5, ExactTerminalVerdict::Miss, i64::MAX);
    let fast_evidence = evidence(
        &fast,
        42,
        8,
        ExactTerminalVerdict::Reached {
            first_hit_tick: 120,
        },
        i64::MIN,
    );
    let miss_b_evidence = evidence(&miss_b, 43, 6, ExactTerminalVerdict::Miss, i64::MIN);
    let ranked = rank_residual_generation(
        &config(&bytes, 1),
        &[
            ResidualGenerationEvaluation {
                compiled: &slow,
                evidence: &slow_evidence,
            },
            ResidualGenerationEvaluation {
                compiled: &miss_a,
                evidence: &miss_a_evidence,
            },
            ResidualGenerationEvaluation {
                compiled: &fast,
                evidence: &fast_evidence,
            },
            ResidualGenerationEvaluation {
                compiled: &miss_b,
                evidence: &miss_b_evidence,
            },
        ],
    )
    .unwrap();
    assert_eq!(ranked.len(), 4);
    assert_eq!(ranked[0], fast.report.candidate_sha256);
    assert_eq!(ranked[1], slow.report.candidate_sha256);
    assert!(ranked.contains(&miss_a.report.candidate_sha256));
    assert!(ranked.contains(&miss_b.report.candidate_sha256));
}

#[test]
fn misses_remain_failure_experience_and_reservoir_prefers_diversity() {
    let (parent, bytes) = parent();
    let mut archive = ResidualOutcomeArchive::new(config(&bytes, 2)).unwrap();
    for (index, behavior) in [30_u8, 30, 31].into_iter().enumerate() {
        let candidate = compiled(&parent, &bytes, index as u64, index as i16 + 1);
        archive
            .record(
                &candidate,
                evidence(
                    &candidate,
                    20 + index as u8,
                    behavior,
                    ExactTerminalVerdict::Miss,
                    i64::MAX,
                ),
            )
            .unwrap();
    }
    assert!(archive.successes().is_empty());
    assert_eq!(archive.failures().len(), 2);
    assert_eq!(
        archive
            .failures()
            .iter()
            .map(|failure| failure.behavior_sha256)
            .collect::<BTreeSet<_>>()
            .len(),
        2
    );
}

#[test]
fn diverse_elites_and_horizon_tightening_require_a_supported_basin() {
    let (parent, bytes) = parent();
    let mut archive = ResidualOutcomeArchive::new(config(&bytes, 8)).unwrap();
    for (index, (tick, behavior)) in [(130, 40), (132, 41), (138, 40), (155, 42)]
        .into_iter()
        .enumerate()
    {
        let candidate = compiled(&parent, &bytes, index as u64, index as i16 + 1);
        archive
            .record(
                &candidate,
                evidence(
                    &candidate,
                    30 + index as u8,
                    behavior,
                    ExactTerminalVerdict::Reached {
                        first_hit_tick: tick,
                    },
                    0,
                ),
            )
            .unwrap();
    }
    let elites = archive.diverse_success_elites(3);
    assert_eq!(
        elites
            .iter()
            .map(|success| success.behavior_sha256)
            .collect::<BTreeSet<_>>()
            .len(),
        3
    );
    let strict = HorizonSupportPolicy {
        minimum_successes: 4,
        minimum_behavior_classes: 3,
        minimum_support_millionths: 1_000_000,
    };
    assert!(archive.horizon_tightening_evidence(145, strict).is_err());
    let supported = archive
        .horizon_tightening_evidence(
            145,
            HorizonSupportPolicy {
                minimum_successes: 3,
                minimum_behavior_classes: 2,
                minimum_support_millionths: 750_000,
            },
        )
        .unwrap();
    assert_eq!(supported.supporting_successes, 3);
    let mut curriculum_archive = ResidualOutcomeArchive::new(config(&bytes, 8)).unwrap();
    for (index, behavior) in [50_u8, 51, 52, 53].into_iter().enumerate() {
        let candidate = compiled(&parent, &bytes, 20 + index as u64, index as i16 + 1);
        curriculum_archive
            .record(
                &candidate,
                evidence(
                    &candidate,
                    60 + index as u8,
                    behavior,
                    ExactTerminalVerdict::Reached {
                        first_hit_tick: 130 + index as u64,
                    },
                    0,
                ),
            )
            .unwrap();
    }
    let off_frontier = compiled(&parent, &bytes, 4, 9);
    curriculum_archive
        .record(
            &off_frontier,
            evidence(
                &off_frontier,
                70,
                54,
                ExactTerminalVerdict::Reached {
                    first_hit_tick: 134,
                },
                0,
            ),
        )
        .unwrap();
    let curriculum = curriculum_archive
        .reverse_curriculum_evidence(
            &parent,
            &bytes,
            16,
            8,
            ReverseCurriculumSupportPolicy {
                initial_tail_ticks: 64,
                expansion_step_ticks: 16,
                minimum_successes: 3,
                minimum_behavior_classes: 3,
                minimum_success_millionths: 800_000,
            },
        )
        .unwrap();
    assert_eq!(curriculum.evaluated_tapes, 5);
    assert_eq!(curriculum.successful_tapes, 4);
    assert_eq!(curriculum.successful_behavior_classes, 4);
    assert_eq!(curriculum.success_millionths, 800_000);
    assert_eq!(curriculum.proposed_start_frame, 8);
    assert!(
        curriculum_archive
            .reverse_curriculum_evidence(
                &parent,
                &bytes,
                8,
                16,
                ReverseCurriculumSupportPolicy {
                    initial_tail_ticks: 64,
                    expansion_step_ticks: 16,
                    minimum_successes: 2,
                    minimum_behavior_classes: 2,
                    minimum_success_millionths: 1,
                },
            )
            .is_err()
    );
}

#[test]
fn minimization_requires_prior_discovery_strict_simplicity_and_exact_replay() {
    let (parent, bytes) = parent();
    let original = {
        let candidate = ResidualCandidate::seal(
            &bytes,
            vec![
                AnalogResidual {
                    port: 0,
                    channel: AnalogChannel::MainX,
                    basis: TemporalBasis::ExactFrame { frame: 3, delta: 5 },
                },
                AnalogResidual {
                    port: 0,
                    channel: AnalogChannel::MainX,
                    basis: TemporalBasis::ExactFrame { frame: 8, delta: 5 },
                },
            ],
            vec![],
        )
        .unwrap();
        compile_residual_candidate(&parent, &bytes, &candidate).unwrap()
    };
    let minimized = compiled(&parent, &bytes, 3, 5);
    let mut archive = ResidualOutcomeArchive::new(config(&bytes, 8)).unwrap();
    assert!(
        archive
            .accept_minimized(
                original.report.realized_tape_sha256,
                &minimized,
                evidence(
                    &minimized,
                    50,
                    50,
                    ExactTerminalVerdict::Reached {
                        first_hit_tick: 130,
                    },
                    0,
                )
            )
            .is_err()
    );
    archive
        .record(
            &original,
            evidence(
                &original,
                51,
                50,
                ExactTerminalVerdict::Reached {
                    first_hit_tick: 130,
                },
                0,
            ),
        )
        .unwrap();
    archive
        .accept_minimized(
            original.report.realized_tape_sha256,
            &minimized,
            evidence(
                &minimized,
                52,
                51,
                ExactTerminalVerdict::Reached {
                    first_hit_tick: 130,
                },
                0,
            ),
        )
        .unwrap();
    assert_eq!(archive.successes().len(), 2);
    assert!(
        archive.successes().iter().any(|success| {
            success.minimized_from == Some(original.report.realized_tape_sha256)
        })
    );
}

#[test]
fn detached_predicates_and_out_of_horizon_hits_fail_closed() {
    let (parent, bytes) = parent();
    let candidate = compiled(&parent, &bytes, 1, 1);
    let mut archive = ResidualOutcomeArchive::new(config(&bytes, 8)).unwrap();
    let mut detached = evidence(&candidate, 60, 60, ExactTerminalVerdict::Miss, 0);
    detached.terminal_program_sha256 = digest(9);
    assert!(archive.record(&candidate, detached).is_err());
    assert!(
        archive
            .record(
                &candidate,
                evidence(
                    &candidate,
                    61,
                    60,
                    ExactTerminalVerdict::Reached {
                        first_hit_tick: 161,
                    },
                    0,
                )
            )
            .is_err()
    );
    assert!(archive.snapshot().unwrap().evaluated_tapes.is_empty());
}

#[test]
fn sealed_snapshot_restores_success_failure_and_dropped_history_exactly() {
    let (parent, bytes) = parent();
    let mut archive = ResidualOutcomeArchive::new(config(&bytes, 2)).unwrap();
    for (index, behavior) in [70_u8, 70, 71].into_iter().enumerate() {
        let candidate = compiled(&parent, &bytes, index as u64, index as i16 + 1);
        archive
            .record(
                &candidate,
                evidence(
                    &candidate,
                    70 + index as u8,
                    behavior,
                    ExactTerminalVerdict::Miss,
                    index as i64,
                ),
            )
            .unwrap();
    }
    let success = compiled(&parent, &bytes, 8, 12);
    archive
        .record(
            &success,
            evidence(
                &success,
                80,
                80,
                ExactTerminalVerdict::Reached {
                    first_hit_tick: 140,
                },
                i64::MIN,
            ),
        )
        .unwrap();
    let snapshot = archive.snapshot().unwrap();
    assert_eq!(snapshot.failures.len(), 2);
    assert_eq!(snapshot.evaluated_tapes.len(), 4);
    assert_eq!(snapshot.evaluation_bindings.len(), 4);
    let bytes = serde_json::to_vec(&snapshot).unwrap();
    let decoded: ResidualRetentionSnapshot = serde_json::from_slice(&bytes).unwrap();
    let restored = ResidualOutcomeArchive::restore(decoded).unwrap();
    assert_eq!(restored.snapshot().unwrap(), snapshot);
    assert_eq!(restored.optimizer_rank(), archive.optimizer_rank());

    let mut tampered = snapshot;
    tampered.successes[0].realized_tape[0] ^= 1;
    tampered.content_sha256 = tampered.compute_identity().unwrap();
    assert!(ResidualOutcomeArchive::restore(tampered).is_err());
}

#[test]
fn evidence_reuse_and_terminal_disagreement_fail_without_partial_mutation() {
    let (parent, bytes) = parent();
    let first = compiled(&parent, &bytes, 2, 4);
    let second = compiled(&parent, &bytes, 3, 5);
    let mut archive = ResidualOutcomeArchive::new(config(&bytes, 8)).unwrap();
    let miss = evidence(&first, 90, 90, ExactTerminalVerdict::Miss, i64::MAX);
    archive.record(&first, miss).unwrap();
    let before = archive.snapshot().unwrap();

    let mut reused = evidence(&second, 90, 91, ExactTerminalVerdict::Miss, i64::MIN);
    reused.episode_sha256 = digest(91);
    assert!(archive.record(&second, reused).is_err());
    assert_eq!(archive.snapshot().unwrap(), before);

    let disagreement = evidence(
        &first,
        92,
        90,
        ExactTerminalVerdict::Reached {
            first_hit_tick: 130,
        },
        0,
    );
    assert!(archive.record(&first, disagreement).is_err());
    assert_eq!(archive.snapshot().unwrap(), before);

    let mut detached = evidence(&second, 93, 91, ExactTerminalVerdict::Miss, 0);
    detached.candidate_sha256 = digest(99);
    assert!(archive.record(&second, detached).is_err());
    assert_eq!(archive.snapshot().unwrap(), before);
}
