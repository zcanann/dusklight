
use super::*;
use crate::action_guidance::{ACTION_GUIDANCE_SCHEMA_V2, movement_action_mask_v2};
use crate::artifact::Digest;
use crate::evaluation_isolation::{EvaluationAttemptInput, EvaluationGenerationSeal};
use crate::offline_rl::{
    canonical_movement_pad_v2, canonical_movement_pad_v3, movement_action_id_v3,
    movement_action_schema_digest_v2, movement_action_schema_digest_v3,
};
use crate::search::SegmentProfile;
use crate::tape::{InputFrame, InputTape, RawPadState};
use crate::transition_corpus::{MacroAction, StateReference, StateReferenceKind, Transition};

fn objective() -> NamedDigest {
    NamedDigest::new("q-search-test", Digest([0xa5; 32]))
}

fn admitted_readiness() -> QProposalReadinessEvidence {
    QProposalReadinessEvidence {
        required_facts_supported: true,
        determinism_proved: true,
        held_out_performance_adequate: true,
        initial_bounded_trial: false,
    }
}

fn corpus_for(candidate: &Candidate) -> TransitionCorpus {
    corpus_for_schema(candidate, MovementActionSchema::V2)
}

fn corpus_for_schema(
    candidate: &Candidate,
    action_schema: MovementActionSchema,
) -> TransitionCorpus {
    let observation_spec = movement_state_v2_spec();
    let feature_count = observation_spec.feature_count();
    let player_present = observation_spec.feature_index("player.present").unwrap();
    let player_is_link = observation_spec.feature_index("player.is_link").unwrap();
    let procedure_present = observation_spec
        .feature_index("player.procedure_present")
        .unwrap();
    let procedure = observation_spec.feature_index("player.procedure").unwrap();
    let position_x = observation_spec.feature_index("player.position_x").unwrap();
    let event_running = observation_spec.feature_index("event.running").unwrap();
    let progress_configured = observation_spec
        .feature_index("objective.progress_configured")
        .unwrap();
    let progress_fraction = observation_spec
        .feature_index("objective.progress_fraction")
        .unwrap();
    let elapsed = observation_spec.feature_index("window.elapsed").unwrap();
    let remaining = observation_spec.feature_index("window.remaining").unwrap();
    let tape = candidate.compile().unwrap();
    let frame_count = tape.frames.len();
    let transitions = tape
        .frames
        .iter()
        .enumerate()
        .map(|(index, frame)| {
            let action_id = action_schema.action_id(frame.pads[0]).unwrap();
            let mut state = vec![0.0; feature_count as usize];
            state[player_present] = 1.0;
            state[player_is_link] = 1.0;
            state[procedure_present] = 1.0;
            state[procedure] = 4.0;
            state[position_x] = index as f32 / 32.0;
            state[event_running] = f32::from(index == 0);
            state[progress_configured] = 1.0;
            state[progress_fraction] = 1.0 / 3.0;
            state[elapsed] = index as f32 / 1024.0;
            state[remaining] = frame_count.saturating_sub(index) as f32 / 1024.0;
            let mut next_state = state.clone();
            next_state[position_x] += 1.0 / 32.0;
            next_state[elapsed] += 1.0 / 1024.0;
            next_state[remaining] = frame_count.saturating_sub(index + 1) as f32 / 1024.0;
            Transition {
                source: StateReference {
                    kind: StateReferenceKind::Boundary,
                    digest: Digest([index as u8 + 1; 32]),
                },
                state,
                action: MacroAction {
                    action_id,
                    macro_kind: action_schema.macro_kind(),
                    parameters: vec![
                        i16::from(frame.pads[0].stick_x),
                        i16::from(frame.pads[0].stick_y),
                        frame.pads[0].buttons as i16,
                    ],
                },
                duration_ticks: 1,
                reward: -1.0,
                next: StateReference {
                    kind: StateReferenceKind::Boundary,
                    digest: Digest([index as u8 + 2; 32]),
                },
                next_state,
                terminal: index + 1 == tape.frames.len(),
            }
        })
        .collect();
    TransitionCorpus::new(
        observation_spec.digest().unwrap(),
        action_schema.digest(),
        feature_count,
        transitions,
    )
    .unwrap()
}

#[test]
fn fitted_q_proposals_are_deterministic_aligned_ordinary_candidates() {
    let disconnected = RawPadState {
        connected: false,
        error: -1,
        ..RawPadState::default()
    };
    let tape = InputTape {
        frames: (0..8)
            .map(|index| InputFrame {
                owned_ports: 1,
                pads: [
                    canonical_movement_pad_v2(if index % 2 == 0 { 0 } else { 18 }).unwrap(),
                    disconnected,
                    disconnected,
                    disconnected,
                ],
                ..InputFrame::default()
            })
            .collect(),
        ..InputTape::default()
    };
    let candidate = Candidate::from_absolute_tape(SegmentProfile::Fsp103ToFsp104, &tape).unwrap();
    let corpus = corpus_for(&candidate);
    let episodes = [QEpisode {
        candidate: candidate.clone(),
        corpus: corpus.clone(),
        outcome: EpisodeOutcomeClass::Successful,
        objective: objective(),
    }];
    let config = QProposalConfig {
        generation: 1,
        max_proposals: 4,
        iterations: 4,
        trees_per_action: 3,
        seed: 7,
        readiness: admitted_readiness(),
    };
    let first = propose_q_candidates(std::slice::from_ref(&corpus), &episodes, config).unwrap();
    let second = propose_q_candidates(&[corpus], &episodes, config).unwrap();
    assert!(!first.candidates.is_empty());
    assert_eq!(first.summary.proposals, first.candidates.len());
    assert_eq!(first.envelopes.len(), first.candidates.len());
    assert!(first.envelopes.iter().all(|envelope| {
        envelope.validate().is_ok()
            && envelope.objective == objective()
            && envelope.action_schema.sha256 == movement_action_schema_digest_v2()
            && envelope.seed == config.seed
    }));
    assert_eq!(
        first.summary.action_guidance_schema,
        ACTION_GUIDANCE_SCHEMA_V2
    );
    assert!(first.summary.state_masked_proposal_states > 0);
    assert_eq!(first.summary.proposal_states, 8);
    let health = first.summary.training_health.as_ref().unwrap();
    assert_eq!(health.update_to_data_ratio, 4.0);
    assert_eq!(
        health.disposition,
        super::super::training_guard::TrainingHealthDisposition::Healthy
    );
    assert_eq!(first.summary.schema, "dusklight-q-proposals/v14");
    assert_eq!(first.summary.step_reward_schema, MOVEMENT_REWARD_SCHEMA_V2);
    assert_eq!(
        first.summary.terminal_reward_schema,
        Q_TERMINAL_REWARD_SCHEMA_V1
    );
    assert_eq!(
        first.summary.learned_parent_policy,
        LEARNED_PARENT_POLICY_V2
    );
    assert_eq!(
        first.summary.initial_trial_budget_policy,
        INITIAL_TRIAL_BUDGET_POLICY_V4
    );
    assert_eq!(first.summary.learned_parent_episodes, 1);
    assert_eq!(first.summary.learned_parent_states, 8);
    assert!(first.summary.dataset_generation_sha256.is_none());
    assert!(first.summary.model_lineage.is_none());
    assert!(first.summary.coverage_gate.learned_policy_enabled);
    assert!(first.summary.proposal_gate.learned_policy_enabled);
    assert_eq!(first.summary.collection_cycle_offset, 3);
    assert!(first.summary.guided_action_evaluations > 0);
    assert_eq!(first.summary.unmasked_q_probe_states, 2);
    assert_eq!(first.summary.unmasked_action_evaluations, 2);
    assert!(first.summary.guided_exploit_interventions > 0);
    assert!(first.summary.temporal_consensus_interventions > 0);
    assert!(first.summary.unmasked_exploratory_interventions > 0);
    assert!(first.summary.structured_counterfactual_interventions > 0);
    assert!(first.summary.archive_novelty_interventions > 0);
    assert!(first.summary.blind_coverage_interventions > 0);
    assert_eq!(
        first
            .candidates
            .iter()
            .map(Candidate::id)
            .collect::<Result<Vec<_>, _>>()
            .unwrap(),
        second
            .candidates
            .iter()
            .map(Candidate::id)
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    );
    assert!(first.candidates.iter().all(|proposal| {
        proposal.validate().is_ok()
            && proposal.ancestry.parent_id.as_deref() == Some(candidate.id().unwrap().as_str())
            && proposal.ancestry.intervention.is_some()
    }));
    assert_eq!(first.summary.coverage.episodes, 1);
    assert_eq!(
        first.summary.coverage.effective_decisions,
        episodes[0].corpus.transitions.len()
    );
    assert_eq!(first.summary.proposer_attribution.len(), 6);
    assert_eq!(
        first.summary.collection_schedule.len(),
        first.summary.proposals
    );
    assert!(
        first
            .summary
            .collection_schedule
            .windows(2)
            .all(|pair| pair[0] != pair[1])
    );
    assert_eq!(
        first.summary.schedule_policy,
        "sparse_action_coverage_safe_majority_with_learned_floor"
    );
    for proposer in [
        "structured_counterfactual",
        "ensemble_disagreement",
        "archive_novelty",
        "blind_coverage",
        "guided_exploit",
        "temporal_consensus",
    ] {
        assert!(
            first
                .summary
                .proposer_attribution
                .iter()
                .any(|item| item.proposer == proposer)
        );
    }
    assert!(
        first
            .summary
            .proposer_attribution
            .iter()
            .any(|item| item.proposer == "ensemble_disagreement" && item.uncertainty_is_heuristic)
    );
    assert!(!first.summary.policy_collapse_audit.single_action_collapse);
    assert_eq!(
        first
            .summary
            .proposer_attribution
            .iter()
            .map(|item| item.requested_budget)
            .sum::<usize>(),
        config.max_proposals
    );
}

#[test]
fn v3_collection_proposes_executable_l_targeting_actions() {
    const BUTTON_L: u16 = 0x0040;
    let disconnected = RawPadState {
        connected: false,
        error: -1,
        ..RawPadState::default()
    };
    let tape = InputTape {
        frames: (0..80)
            .map(|_| InputFrame {
                owned_ports: 1,
                pads: [
                    canonical_movement_pad_v3(0).unwrap(),
                    disconnected,
                    disconnected,
                    disconnected,
                ],
                ..InputFrame::default()
            })
            .collect(),
        ..InputTape::default()
    };
    let candidate = Candidate::from_absolute_tape(SegmentProfile::Fsp103ToFsp104, &tape).unwrap();
    let corpus = corpus_for_schema(&candidate, MovementActionSchema::V3);
    let episodes = [QEpisode {
        candidate,
        corpus: corpus.clone(),
        outcome: EpisodeOutcomeClass::Successful,
        objective: objective(),
    }];
    let batch = propose_q_candidates(
        &[corpus],
        &episodes,
        QProposalConfig {
            generation: 1,
            max_proposals: 480,
            iterations: 1,
            trees_per_action: 1,
            seed: 7,
            readiness: QProposalReadinessEvidence {
                required_facts_supported: false,
                determinism_proved: false,
                held_out_performance_adequate: false,
                initial_bounded_trial: false,
            },
        },
    )
    .unwrap();

    assert_eq!(
        batch.summary.action_guidance_schema,
        ACTION_GUIDANCE_SCHEMA_V3
    );
    assert_eq!(batch.summary.policy_collapse_audit.action_catalog_size, 136);
    assert!(batch.envelopes.iter().all(|envelope| {
        envelope.action_schema.id == "movement-action/v3"
            && envelope.action_schema.sha256 == movement_action_schema_digest_v3()
    }));
    assert!(batch.candidates.iter().any(|candidate| {
        candidate
            .compile()
            .unwrap()
            .frames
            .iter()
            .any(|frame| frame.pads[0].buttons & BUTTON_L != 0)
    }));
    for candidate in &batch.candidates {
        for frame in candidate.compile().unwrap().frames {
            let action = movement_action_id_v3(frame.pads[0]).unwrap();
            assert_eq!(
                canonical_movement_pad_v3(action).unwrap().buttons,
                frame.pads[0].buttons
            );
        }
    }
}

#[test]
fn route_q_projection_values_success_and_failed_tape_endings() {
    let disconnected = RawPadState {
        connected: false,
        error: -1,
        ..RawPadState::default()
    };
    let tape = InputTape {
        frames: vec![InputFrame {
            owned_ports: 1,
            pads: [
                canonical_movement_pad_v2(0).unwrap(),
                disconnected,
                disconnected,
                disconnected,
            ],
            ..InputFrame::default()
        }],
        ..InputTape::default()
    };
    let candidate = Candidate::from_absolute_tape(SegmentProfile::Fsp103ToFsp104, &tape).unwrap();
    let corpus = corpus_for(&candidate);
    let successful = route_q_training_transition(&corpus.transitions[0], true);
    assert!(successful.terminal);
    assert_eq!(successful.reward, -1.0 + Q_GOAL_TERMINAL_ADJUSTMENT);

    let mut missed = corpus.transitions[0].clone();
    missed.terminal = false;
    let failed = route_q_training_transition(&missed, true);
    assert!(failed.terminal);
    assert_eq!(failed.reward, -1.0 + Q_FAILURE_TERMINAL_ADJUSTMENT);

    let interior = route_q_training_transition(&missed, false);
    assert!(!interior.terminal);
    assert_eq!(interior.reward, -1.0);
}

#[test]
fn authenticated_progress_failure_is_a_learned_repair_parent() {
    let disconnected = RawPadState {
        connected: false,
        error: -1,
        ..RawPadState::default()
    };
    let tape = InputTape {
        frames: (0..8)
            .map(|index| InputFrame {
                owned_ports: 1,
                pads: [
                    canonical_movement_pad_v2(if index % 2 == 0 { 0 } else { 18 }).unwrap(),
                    disconnected,
                    disconnected,
                    disconnected,
                ],
                ..InputFrame::default()
            })
            .collect(),
        ..InputTape::default()
    };
    let candidate = Candidate::from_absolute_tape(SegmentProfile::Fsp103ToFsp104, &tape).unwrap();
    let mut corpus = corpus_for(&candidate);
    let progress = movement_state_v2_spec()
        .feature_index("objective.progress_fraction")
        .unwrap();
    for (index, transition) in corpus.transitions.iter_mut().enumerate() {
        if index >= 3 {
            transition.state[progress] = 2.0 / 3.0;
        }
        if index >= 2 {
            transition.next_state[progress] = 2.0 / 3.0;
        }
    }
    corpus.transitions.last_mut().unwrap().terminal = false;
    corpus.validate().unwrap();
    let batch = propose_q_candidates(
        std::slice::from_ref(&corpus),
        &[QEpisode {
            candidate: candidate.clone(),
            corpus: corpus.clone(),
            outcome: EpisodeOutcomeClass::Failed,
            objective: objective(),
        }],
        QProposalConfig {
            generation: 1,
            max_proposals: 4,
            iterations: 4,
            trees_per_action: 3,
            seed: 77,
            readiness: admitted_readiness(),
        },
    )
    .unwrap();
    assert_eq!(batch.summary.learned_parent_episodes, 1);
    assert_eq!(batch.summary.learned_parent_states, 8);
    assert!(batch.summary.guided_exploit_interventions > 0);
    assert!(batch.summary.temporal_consensus_interventions > 0);
    assert!(batch.candidates.iter().any(|proposal| {
        proposal.ancestry.parent_id.as_deref() == Some(candidate.id().unwrap().as_str())
            && proposal
                .ancestry
                .mutation
                .as_deref()
                .is_some_and(|mutation| mutation.starts_with("q_"))
    }));
}

#[test]
fn online_q_binds_deterministic_model_to_immutable_dataset_generation() {
    let disconnected = RawPadState {
        connected: false,
        error: -1,
        ..RawPadState::default()
    };
    let tape = InputTape {
        frames: (0..4)
            .map(|index| InputFrame {
                owned_ports: 1,
                pads: [
                    canonical_movement_pad_v2(if index % 2 == 0 { 0 } else { 18 }).unwrap(),
                    disconnected,
                    disconnected,
                    disconnected,
                ],
                ..InputFrame::default()
            })
            .collect(),
        ..InputTape::default()
    };
    let candidate = Candidate::from_absolute_tape(SegmentProfile::Fsp103ToFsp104, &tape).unwrap();
    let corpus = corpus_for(&candidate);
    let corpus_digest = corpus.content_digest().unwrap();
    let seal = EvaluationGenerationSeal::build(
        0,
        2,
        2,
        2,
        0,
        &[
            EvaluationAttemptInput {
                candidate_id: "candidate-a".into(),
                attempt: 1,
                worker_id: "evaluation/worker-0".into(),
                transition_corpus_sha256: Some(corpus_digest),
            },
            EvaluationAttemptInput {
                candidate_id: "candidate-a".into(),
                attempt: 2,
                worker_id: "evaluation/worker-1".into(),
                transition_corpus_sha256: None,
            },
        ],
    )
    .unwrap();
    let dataset =
        OnlineDatasetGeneration::build(None, &seal, std::slice::from_ref(&corpus)).unwrap();
    let episodes = [QEpisode {
        candidate,
        corpus: corpus.clone(),
        outcome: EpisodeOutcomeClass::Successful,
        objective: objective(),
    }];
    let config = QProposalConfig {
        generation: 1,
        max_proposals: 2,
        iterations: 2,
        trees_per_action: 3,
        seed: 9,
        readiness: admitted_readiness(),
    };
    let first = propose_q_candidates_with_lineage(
        std::slice::from_ref(&corpus),
        &episodes,
        config,
        &dataset,
        None,
    )
    .unwrap();
    let second =
        propose_q_candidates_with_lineage(&[corpus], &episodes, config, &dataset, None).unwrap();
    assert_eq!(
        first.summary.dataset_generation_sha256,
        Some(dataset.generation_sha256)
    );
    let lineage = first.summary.model_lineage.as_ref().unwrap();
    lineage.validate().unwrap();
    assert_eq!(
        second.summary.model_lineage.as_ref().unwrap(),
        lineage,
        "same immutable dataset and training config must reproduce exact model lineage"
    );
}

#[test]
fn inadequate_action_support_reassigns_learned_budget_to_safe_fallbacks() {
    let disconnected = RawPadState {
        connected: false,
        error: -1,
        ..RawPadState::default()
    };
    let tape = InputTape {
        frames: (0..4)
            .map(|_| InputFrame {
                owned_ports: 1,
                pads: [
                    canonical_movement_pad_v2(0).unwrap(),
                    disconnected,
                    disconnected,
                    disconnected,
                ],
                ..InputFrame::default()
            })
            .collect(),
        ..InputTape::default()
    };
    let candidate = Candidate::from_absolute_tape(SegmentProfile::Fsp103ToFsp104, &tape).unwrap();
    let corpus = corpus_for(&candidate);
    let batch = propose_q_candidates(
        std::slice::from_ref(&corpus),
        &[QEpisode {
            candidate,
            corpus: corpus.clone(),
            outcome: EpisodeOutcomeClass::Successful,
            objective: objective(),
        }],
        QProposalConfig {
            generation: 1,
            max_proposals: 3,
            iterations: 2,
            trees_per_action: 3,
            seed: 9,
            readiness: admitted_readiness(),
        },
    )
    .unwrap();
    assert_eq!(
        batch.summary.coverage_gate.disposition,
        super::super::training_guard::CoverageDisposition::FallbackInsufficientActionSupport
    );
    assert!(!batch.summary.coverage_gate.learned_policy_enabled);
    assert_eq!(
        batch.summary.coverage_gate.fallback_policy,
        Some("structured_archive_blind_only")
    );
    assert!(batch.summary.model_lineage.is_none());
    assert!(batch.summary.training_health.is_none());
    assert_eq!(
        batch.summary.schedule_policy,
        "readiness_fallback_structured_archive_blind_round_robin"
    );
    assert!(
        batch
            .summary
            .collection_schedule
            .iter()
            .all(|lane| matches!(
                *lane,
                "structured_counterfactual" | "archive_novelty" | "blind_coverage"
            ))
    );
    assert_eq!(
        batch
            .summary
            .proposer_attribution
            .iter()
            .take(3)
            .map(|lane| lane.requested_budget)
            .sum::<usize>(),
        0
    );
}

#[test]
fn unsupported_facts_unproved_determinism_and_bad_holdout_disable_learning() {
    let disconnected = RawPadState {
        connected: false,
        error: -1,
        ..RawPadState::default()
    };
    let tape = InputTape {
        frames: (0..8)
            .map(|index| InputFrame {
                owned_ports: 1,
                pads: [
                    canonical_movement_pad_v2(if index % 2 == 0 { 0 } else { 18 }).unwrap(),
                    disconnected,
                    disconnected,
                    disconnected,
                ],
                ..InputFrame::default()
            })
            .collect(),
        ..InputTape::default()
    };
    let candidate = Candidate::from_absolute_tape(SegmentProfile::Fsp103ToFsp104, &tape).unwrap();
    let corpus = corpus_for(&candidate);
    let batch = propose_q_candidates(
        std::slice::from_ref(&corpus),
        &[QEpisode {
            candidate,
            corpus: corpus.clone(),
            outcome: EpisodeOutcomeClass::Successful,
            objective: objective(),
        }],
        QProposalConfig {
            generation: 1,
            max_proposals: 3,
            iterations: 2,
            trees_per_action: 3,
            seed: 9,
            readiness: QProposalReadinessEvidence {
                required_facts_supported: false,
                determinism_proved: false,
                held_out_performance_adequate: false,
                initial_bounded_trial: false,
            },
        },
    )
    .unwrap();
    assert!(batch.summary.coverage_gate.learned_policy_enabled);
    assert!(!batch.summary.proposal_gate.learned_policy_enabled);
    assert_eq!(
        batch.summary.proposal_gate.blockers,
        [
            super::super::training_guard::LearnedProposalBlocker::RequiredFactsUnsupported,
            super::super::training_guard::LearnedProposalBlocker::DeterminismUnproved,
        ]
    );
    assert!(batch.summary.model_lineage.is_none());
    assert!(batch.summary.training_health.is_none());
    assert!(
        batch
            .summary
            .collection_schedule
            .iter()
            .all(|lane| matches!(
                *lane,
                "structured_counterfactual" | "archive_novelty" | "blind_coverage"
            ))
    );
}

#[test]
fn remainder_budget_rotates_across_all_collection_lanes() {
    assert_eq!(split_proposer_budget(3, 0), [1, 1, 1, 0, 0, 0]);
    assert_eq!(split_proposer_budget(3, 1), [0, 1, 1, 1, 0, 0]);
    assert_eq!(split_proposer_budget(3, 4), [1, 0, 0, 0, 1, 1]);
    let mut totals = [0; 6];
    for generation in 0..6 {
        for (total, budget) in totals.iter_mut().zip(split_proposer_budget(3, generation)) {
            *total += budget;
        }
    }
    assert_eq!(totals, [3; 6]);
    assert_eq!(split_sparse_action_budget(3), [0, 0, 0, 1, 1, 1]);
    assert_eq!(split_sparse_action_budget(10), [1, 1, 0, 3, 2, 3]);
    assert_eq!(split_sparse_action_budget(12), [1, 1, 1, 3, 3, 3]);
}

#[test]
fn proposer_budget_allocator_exactly_fills_small_and_large_populations() {
    let assert_exact_allocation = |total: usize, cycle_offset: usize| {
        let budgets = split_proposer_budget(total, cycle_offset);
        assert_eq!(budgets.iter().sum::<usize>(), total);

        let base = total / budgets.len();
        let remainder = total % budgets.len();
        assert!(
            budgets
                .iter()
                .all(|budget| *budget == base || *budget == base + 1)
        );
        assert_eq!(
            budgets.iter().filter(|budget| **budget == base + 1).count(),
            remainder
        );
    };

    for total in 0..=512 {
        for cycle_offset in 0..6 {
            assert_exact_allocation(total, cycle_offset);
        }
    }
    for total in [1_000, 16_384, 1_000_000, 10_000_003] {
        for cycle_offset in 0..6 {
            assert_exact_allocation(total, cycle_offset);
        }
    }
}

#[test]
fn initial_trial_is_capped_and_cannot_bypass_fact_or_determinism_gates() {
    let ready_coverage = OnlineCoverageGate::evaluate(
        8,
        &BTreeMap::from([(0, 4), (18, 4)]),
        8,
        CoverageGuardConfig::default(),
    )
    .unwrap();
    let admitted = LearnedProposalGate::evaluate(&ready_coverage, true, true, false, true);
    assert!(admitted.learned_policy_enabled);
    assert_eq!(split_initial_learned_budget(2), [1, 1, 0, 0, 0, 0]);
    assert_eq!(split_initial_learned_budget(3), [1, 1, 1, 0, 0, 0]);
    assert_eq!(split_initial_learned_budget(6), [1, 1, 1, 1, 1, 1]);
    assert_eq!(split_initial_learned_budget(9), [2, 2, 2, 1, 1, 1]);
    assert_eq!(split_initial_learned_budget(64), [21, 20, 20, 1, 1, 1]);

    let unsupported = LearnedProposalGate::evaluate(&ready_coverage, false, true, false, true);
    assert!(!unsupported.learned_policy_enabled);
    assert!(
        unsupported.blockers.contains(
            &super::super::training_guard::LearnedProposalBlocker::RequiredFactsUnsupported
        )
    );
    let nondeterministic = LearnedProposalGate::evaluate(&ready_coverage, true, false, false, true);
    assert!(!nondeterministic.learned_policy_enabled);
    assert!(
        nondeterministic
            .blockers
            .contains(&super::super::training_guard::LearnedProposalBlocker::DeterminismUnproved)
    );
}

#[test]
fn guidance_lane_prefers_mask_while_exploration_lane_remains_unmasked() {
    let mut state = vec![0.0; 98];
    state[15] = 1.0;
    state[16] = 1.0;
    state[37] = 1.0;
    let guidance = movement_action_mask_v2(&state).unwrap();
    let masked_high_value = QEstimate {
        action: 67,
        mean: 10.0,
        variance: 4.0,
    };
    let recommended_lower_value = QEstimate {
        action: 0,
        mean: 1.0,
        variance: 0.0,
    };
    let ranked = [masked_high_value, recommended_lower_value];
    assert_eq!(
        guided_exploit(&ranked, &guidance).unwrap().action,
        recommended_lower_value.action
    );
    assert_eq!(
        unmasked_explore(&ranked, 0.0).unwrap().action,
        masked_high_value.action
    );
    assert!(guided_exploit(&[masked_high_value], &guidance).is_none());
    assert_eq!(
        unmasked_explore(&[masked_high_value], 0.0).unwrap().action,
        masked_high_value.action
    );
}

#[test]
fn online_q_rejects_excessive_update_to_data_ratio_before_proposing() {
    let disconnected = RawPadState {
        connected: false,
        error: -1,
        ..RawPadState::default()
    };
    let tape = InputTape {
        frames: vec![InputFrame {
            owned_ports: 1,
            pads: [
                canonical_movement_pad_v2(0).unwrap(),
                disconnected,
                disconnected,
                disconnected,
            ],
            ..InputFrame::default()
        }],
        ..InputTape::default()
    };
    let candidate = Candidate::from_absolute_tape(SegmentProfile::Fsp103ToFsp104, &tape).unwrap();
    let corpus = corpus_for(&candidate);
    let error = propose_q_candidates(
        std::slice::from_ref(&corpus),
        &[QEpisode {
            candidate,
            corpus: corpus.clone(),
            outcome: EpisodeOutcomeClass::Successful,
            objective: objective(),
        }],
        QProposalConfig {
            generation: 0,
            max_proposals: 1,
            iterations: 33,
            trees_per_action: 1,
            seed: 1,
            readiness: admitted_readiness(),
        },
    )
    .unwrap_err();
    assert!(error.to_string().contains("update-to-data ratio"));
}
