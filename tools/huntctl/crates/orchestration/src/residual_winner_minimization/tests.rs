use super::*;
use dusklight_automation_contracts::tape::{InputFrame, RawPadState, TapeBoot};
use dusklight_search::residual_action::{AnalogChannel, AnalogResidual, TemporalBasis};
use dusklight_search::residual_retention::{
    FailureRetentionPolicy, ResidualOutcomeArchive, ResidualRetentionConfig,
};

fn parent() -> (InputTape, Vec<u8>) {
    let tape = InputTape {
        boot: TapeBoot::Process,
        tick_rate_numerator: 30,
        tick_rate_denominator: 1,
        frames: vec![
            InputFrame {
                owned_ports: 1,
                pads: [
                    RawPadState {
                        connected: true,
                        ..RawPadState::default()
                    },
                    RawPadState::default(),
                    RawPadState::default(),
                    RawPadState::default(),
                ],
                ..InputFrame::default()
            };
            8
        ],
    };
    let bytes = tape.encode().unwrap();
    (tape, bytes)
}

#[test]
fn component_reductions_are_strict_deterministic_and_never_empty() {
    let (parent, bytes) = parent();
    let candidate = ResidualCandidate::seal(
        &bytes,
        vec![
            AnalogResidual {
                port: 0,
                channel: AnalogChannel::MainX,
                basis: TemporalBasis::ExactFrame {
                    frame: 1,
                    delta: 10,
                },
            },
            AnalogResidual {
                port: 0,
                channel: AnalogChannel::MainX,
                basis: TemporalBasis::ExactFrame {
                    frame: 5,
                    delta: -10,
                },
            },
        ],
        vec![],
    )
    .unwrap();
    let compiled = compile_residual_candidate_to_horizon(&parent, &bytes, &candidate, 8).unwrap();
    let complexity = tape_input_complexity(&compiled.tape);
    let mut seen = BTreeSet::new();
    let proposals =
        reduction_proposals(&parent, &bytes, 8, &candidate, complexity, 2, &mut seen).unwrap();
    assert_eq!(proposals.len(), 2);
    assert!(proposals.iter().all(|proposal| {
        proposal.candidate.analog.len() == 1
            && proposal.candidate.buttons.is_empty()
            && proposal.input_complexity < complexity
    }));
    let repeated =
        reduction_proposals(&parent, &bytes, 8, &candidate, complexity, 2, &mut seen).unwrap();
    assert!(repeated.is_empty());
}

#[test]
fn component_reductions_skip_inert_and_incumbent_equivalent_subsets() {
    let (parent, bytes) = parent();
    let candidate = ResidualCandidate::seal(
        &bytes,
        vec![AnalogResidual {
            port: 0,
            channel: AnalogChannel::MainX,
            basis: TemporalBasis::ExactFrame {
                frame: 1,
                delta: 10,
            },
        }],
        vec![dusklight_search::residual_action::ButtonResidual {
            port: 0,
            buttons: 1,
            start_frame: 2,
            duration_frames: 1,
            mode: dusklight_search::residual_action::ButtonResidualMode::Release,
        }],
    )
    .unwrap();
    let compiled = compile_residual_candidate_to_horizon(&parent, &bytes, &candidate, 8).unwrap();
    let proposals = reduction_proposals(
        &parent,
        &bytes,
        8,
        &candidate,
        tape_input_complexity(&compiled.tape),
        2,
        &mut BTreeSet::new(),
    )
    .unwrap();
    assert!(proposals.is_empty());
}

#[test]
fn sealed_minimized_candidate_rejects_detachment() {
    let (parent, bytes) = parent();
    let candidate = ResidualCandidate::seal(
        &bytes,
        vec![AnalogResidual {
            port: 0,
            channel: AnalogChannel::MainX,
            basis: TemporalBasis::ExactFrame {
                frame: 1,
                delta: 10,
            },
        }],
        vec![],
    )
    .unwrap();
    let compiled = compile_residual_candidate_to_horizon(&parent, &bytes, &candidate, 8).unwrap();
    let source = ArtifactReference {
        path: "build/campaigns/source/candidate.json".into(),
        sha256: Digest([7; 32]),
    };
    let mut artifact =
        ResidualMinimizedCandidate::seal(source, Digest([9; 32]), candidate, compiled.report)
            .unwrap();
    artifact.compilation.realized_tape_sha256 = Digest([3; 32]);
    artifact.content_sha256 = artifact.identity().unwrap();
    artifact.validate().unwrap();
    assert!(artifact.validate_against(&parent, &bytes, 8).is_err());
}

#[test]
fn exact_replay_evidence_requires_identical_terminal_repetitions() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../..")
        .canonicalize()
        .unwrap();
    let optimization: OptimizationRequest = serde_json::from_slice(
        &fs::read(root.join(
            "routes/Glitch Exhibition/intro/benchmarks/ordon-q125-residual-campaign.request.json",
        ))
        .unwrap(),
    )
    .unwrap();
    let (parent, bytes) = parent();
    let candidate = ResidualCandidate::seal(
        &bytes,
        vec![AnalogResidual {
            port: 0,
            channel: AnalogChannel::MainX,
            basis: TemporalBasis::ExactFrame {
                frame: 1,
                delta: 10,
            },
        }],
        vec![],
    )
    .unwrap();
    let compiled = compile_residual_candidate_to_horizon(&parent, &bytes, &candidate, 8).unwrap();
    let proposal = ReductionProposal {
        id: "min-test".into(),
        candidate,
        input_complexity: tape_input_complexity(&compiled.tape),
        compiled,
    };
    let attempt = |repetition, boundary: &str| NativeResidualAttempt {
        repetition,
        worker_seed: 1,
        wire_candidate_id: format!("min-test-r{repetition:03}"),
        batch_request: ArtifactReference {
            path: format!("build/request-{repetition}.json"),
            sha256: Digest([1; 32]),
        },
        batch_result: ArtifactReference {
            path: format!("build/result-{repetition}.json"),
            sha256: Digest([2; 32]),
        },
        episode_shard: ArtifactReference {
            path: format!("build/episode-{repetition}.dseps"),
            sha256: Digest([3; 32]),
        },
        restore_identity: "4".repeat(32),
        checkpoint_bytes: 1,
        simulated_ticks: 120,
        first_hit_tick: Some(120),
        terminal_boundary_fingerprint: boundary.into(),
        behavior_sha256: Digest([5; 32]),
    };
    let reached = exact_replay_evidence(&optimization, &proposal, &[attempt(1, "a")]).unwrap();
    assert_eq!(
        reached.verdict,
        ExactTerminalVerdict::Reached {
            first_hit_tick: 120
        }
    );
    let mut repeated = optimization;
    repeated.execution.repetitions = 2;
    assert!(
        exact_replay_evidence(&repeated, &proposal, &[attempt(1, "a"), attempt(2, "b")],).is_err()
    );
}

#[test]
fn summary_reproduces_every_accepted_reduction_and_rejects_resealed_drift() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../..")
        .canonicalize()
        .unwrap();
    let optimization: OptimizationRequest = serde_json::from_slice(
        &fs::read(root.join(
            "routes/Glitch Exhibition/intro/benchmarks/ordon-q125-residual-campaign.request.json",
        ))
        .unwrap(),
    )
    .unwrap();
    let (parent, bytes) = parent();
    let source_candidate = ResidualCandidate::seal(
        &bytes,
        vec![
            AnalogResidual {
                port: 0,
                channel: AnalogChannel::MainX,
                basis: TemporalBasis::ExactFrame {
                    frame: 1,
                    delta: 10,
                },
            },
            AnalogResidual {
                port: 0,
                channel: AnalogChannel::MainX,
                basis: TemporalBasis::ExactFrame {
                    frame: 5,
                    delta: -10,
                },
            },
        ],
        vec![],
    )
    .unwrap();
    let source =
        compile_residual_candidate_to_horizon(&parent, &bytes, &source_candidate, 8).unwrap();
    let source_complexity = tape_input_complexity(&source.tape);
    let mut seen = BTreeSet::new();
    let mut reductions = reduction_proposals(
        &parent,
        &bytes,
        8,
        &source_candidate,
        source_complexity,
        2,
        &mut seen,
    )
    .unwrap();
    let minimized = reductions.remove(0);
    let rejected = reductions.remove(0);
    let mut archive = ResidualOutcomeArchive::new(ResidualRetentionConfig {
        parent_tape_sha256: source_candidate.parent_tape_sha256,
        terminal_program_sha256: optimization.terminal_predicate.program_sha256,
        terminal_definition_sha256: optimization.terminal_predicate.definition_sha256,
        exploration_horizon_ticks: 8,
        promotion_before_tick: 5,
        maximum_candidates: 10,
        failures: FailureRetentionPolicy::All,
    })
    .unwrap();
    let evidence = |candidate: &CompiledResidualCandidate, byte: u8| ResidualEvaluationEvidence {
        candidate_sha256: candidate.report.candidate_sha256,
        realized_tape_sha256: candidate.report.realized_tape_sha256,
        terminal_program_sha256: optimization.terminal_predicate.program_sha256,
        terminal_definition_sha256: optimization.terminal_predicate.definition_sha256,
        evaluation_sha256: Digest([byte; 32]),
        episode_sha256: Digest([byte.saturating_add(1); 32]),
        behavior_sha256: Digest([byte.saturating_add(2); 32]),
        verdict: ExactTerminalVerdict::Reached { first_hit_tick: 6 },
        shaped_progress_millionths: None,
        native_risk_events: None,
    };
    archive.record(&source, evidence(&source, 10)).unwrap();
    let attempt = NativeResidualAttempt {
        repetition: 1,
        worker_seed: 1,
        wire_candidate_id: format!("{}-r001", minimized.id),
        batch_request: ArtifactReference {
            path: "build/min/request.json".into(),
            sha256: Digest([20; 32]),
        },
        batch_result: ArtifactReference {
            path: "build/min/result.json".into(),
            sha256: Digest([21; 32]),
        },
        episode_shard: ArtifactReference {
            path: "build/min/episode.dseps".into(),
            sha256: Digest([22; 32]),
        },
        restore_identity: "4".repeat(32),
        checkpoint_bytes: 1,
        simulated_ticks: 6,
        first_hit_tick: Some(6),
        terminal_boundary_fingerprint: "5".repeat(32),
        behavior_sha256: Digest([23; 32]),
    };
    let rejected_attempt = NativeResidualAttempt {
        repetition: 1,
        worker_seed: 1,
        wire_candidate_id: format!("{}-r001", rejected.id),
        batch_request: ArtifactReference {
            path: "build/min/rejected-request.json".into(),
            sha256: Digest([24; 32]),
        },
        batch_result: ArtifactReference {
            path: "build/min/rejected-result.json".into(),
            sha256: Digest([25; 32]),
        },
        episode_shard: ArtifactReference {
            path: "build/min/rejected-episode.dseps".into(),
            sha256: Digest([26; 32]),
        },
        restore_identity: "4".repeat(32),
        checkpoint_bytes: 1,
        simulated_ticks: 7,
        first_hit_tick: None,
        terminal_boundary_fingerprint: "6".repeat(32),
        behavior_sha256: Digest([27; 32]),
    };
    let minimized_evidence =
        exact_replay_evidence(&optimization, &minimized, std::slice::from_ref(&attempt)).unwrap();
    archive
        .accept_minimized(
            source.report.realized_tape_sha256,
            &minimized.compiled,
            minimized_evidence,
        )
        .unwrap();
    let reference = |path: &str, byte| ArtifactReference {
        path: path.into(),
        sha256: Digest([byte; 32]),
    };
    let mut second_final_attempt = attempt.clone();
    second_final_attempt.repetition = 2;
    second_final_attempt.wire_candidate_id = "final-proof-r002".into();
    let final_exact_replays = vec![attempt.clone(), second_final_attempt];
    let mut summary = ResidualWinnerMinimizationSummary {
        schema: RESIDUAL_WINNER_MINIMIZATION_SCHEMA_V3.into(),
        content_sha256: Digest::ZERO,
        status: ResidualWinnerMinimizationStatus::Minimized,
        optimization_request_sha256: optimization.content_sha256,
        execution_binding_sha256: Digest([30; 32]),
        source_request: reference("routes/request.json", 31),
        source_execution: reference("build/source/execution.json", 32),
        source_checkpoint: reference("build/source/checkpoint.json", 33),
        source_candidate: reference("build/source/candidate.json", 34),
        discovered_candidate_sha256: source_candidate.content_sha256,
        discovered_tape_sha256: source.report.realized_tape_sha256,
        discovered_first_hit_tick: 6,
        discovered_input_complexity: source_complexity,
        minimized_candidate_sha256: minimized.candidate.content_sha256,
        minimized_tape_sha256: minimized.compiled.report.realized_tape_sha256,
        minimized_first_hit_tick: 6,
        minimized_input_complexity: minimized.input_complexity,
        evaluated_candidates: 2,
        candidate_budget: 3,
        accepted_reduction_count: 1,
        charged_simulated_ticks: 25,
        minimized_candidate: Some(reference("build/min/minimized.candidate.json", 35)),
        minimized_tape: Some(reference("build/min/minimized.tape", 36)),
        evaluations: vec![
            ResidualReductionEvaluation {
                round: 0,
                candidate: minimized.candidate,
                compilation: minimized.compiled.report,
                input_complexity: minimized.input_complexity,
                first_hit_tick: Some(6),
                accepted: true,
                exact_replays: vec![attempt],
            },
            ResidualReductionEvaluation {
                round: 0,
                candidate: rejected.candidate,
                compilation: rejected.compiled.report,
                input_complexity: rejected.input_complexity,
                first_hit_tick: None,
                accepted: false,
                exact_replays: vec![rejected_attempt],
            },
        ],
        final_replay_process_mode: "cold_process_per_repetition".into(),
        final_exact_replays,
        retention: archive.snapshot().unwrap(),
    };
    summary.content_sha256 = summary.identity().unwrap();
    summary.validate().unwrap();

    let mut process_mode_drift = summary.clone();
    process_mode_drift.final_replay_process_mode = "persistent_process".into();
    process_mode_drift.content_sha256 = process_mode_drift.identity().unwrap();
    assert!(process_mode_drift.validate().is_err());

    let mut final_proof_drift = summary.clone();
    final_proof_drift.final_exact_replays[1].first_hit_tick = Some(7);
    final_proof_drift.content_sha256 = final_proof_drift.identity().unwrap();
    assert!(final_proof_drift.validate().is_err());

    summary.evaluations[0].input_complexity = source_complexity;
    summary.content_sha256 = summary.identity().unwrap();
    assert!(summary.validate().is_err());
}
