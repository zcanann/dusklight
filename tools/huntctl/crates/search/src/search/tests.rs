use super::*;

#[test]
fn tape_intervention_is_minimal_and_requires_shared_identity() {
    let parent = Candidate::baseline(SegmentProfile::BootToFsp103)
        .compile()
        .unwrap();
    let mut child = parent.clone();
    child.frames[1].pads[0].buttons ^= BUTTON_A;
    assert_eq!(
        tape_intervention(&parent, &child),
        Some(InterventionRange {
            start_frame: 1,
            end_frame_exclusive: 2,
            parent_end_frame_exclusive: 2,
        })
    );
    assert_eq!(tape_intervention(&parent, &parent), None);
    child.boot = TapeBoot::Stage {
        stage: "F_SP103".into(),
        room: 1,
        point: 1,
        layer: 3,
        save_slot: None,
        fixture: None,
    };
    assert_eq!(tape_intervention(&parent, &child), None);
}

#[test]
fn macro_ir_compiles_analog_roll_and_press() {
    let candidate = Candidate {
        schema: CANDIDATE_SCHEMA.into(),
        segment: SegmentProfile::Fsp103ToFsp104,
        boot: TapeBoot::Process,
        actions: vec![
            MacroAction::Move {
                angle_degrees: 90,
                magnitude: 127,
                frames: 2,
            },
            MacroAction::Roll {
                angle_degrees: 0,
                magnitude: 100,
                button_frame: 1,
                recovery_frames: 2,
                spacing: RollSpacing {
                    period_ticks: 4,
                    phase_tick: 3,
                },
            },
            MacroAction::Press {
                buttons: vec![ControllerButton::Start],
                hold_frames: 1,
                neutral_frames: 1,
            },
        ],
        ancestry: Ancestry::default(),
    };
    let tape = candidate.compile().unwrap();
    assert_eq!(tape.frames.len(), 8);
    assert_eq!(tape.frames[0].pads[0].stick_x, 127);
    assert_eq!(tape.frames[0].pads[0].stick_y, 0);
    assert_eq!(tape.frames[2].pads[0].buttons, 0);
    assert_eq!(tape.frames[3].pads[0].buttons, BUTTON_B);
    assert_eq!(tape.frames[3].pads[0].stick_y, 100);
    assert_eq!(tape.frames[6].pads[0].buttons, BUTTON_START);
    assert_eq!(tape.frames[7].pads[0].buttons, 0);

    let mut wrong_phase = candidate.clone();
    let MacroAction::Roll { spacing, .. } = &mut wrong_phase.actions[1] else {
        unreachable!()
    };
    spacing.phase_tick = 2;
    assert!(matches!(
        wrong_phase.compile(),
        Err(SearchError::NonCanonicalTape(_))
    ));
}

#[test]
fn legacy_roll_json_defaults_to_first_frame_and_unconstrained_phase() {
    let action: MacroAction = serde_json::from_str(
        r#"{"op":"roll","angle_degrees":0,"magnitude":100,"recovery_frames":2}"#,
    )
    .unwrap();
    assert_eq!(
        action,
        MacroAction::Roll {
            angle_degrees: 0,
            magnitude: 100,
            button_frame: 0,
            recovery_frames: 2,
            spacing: RollSpacing::default(),
        }
    );
}

#[test]
fn typed_game_tactic_is_a_first_class_static_search_macro() {
    let plan = GameTacticPlan::new(crate::game_tactic::GameTactic::Crawl {
        direction_degrees: 90,
        magnitude: 80,
        frames: 3,
        action_held: true,
    });
    let candidate = Candidate {
        schema: CANDIDATE_SCHEMA.into(),
        segment: SegmentProfile::LinkControlToTunnelCrawlStart,
        boot: TapeBoot::Process,
        actions: vec![MacroAction::GameTactic { plan: plan.clone() }],
        ancestry: Ancestry::default(),
    };
    let tape = candidate.compile().unwrap();
    assert_eq!(tape.frames.len(), 3);
    assert_eq!(tape.frames[0].pads[0].stick_x, 80);
    assert_eq!(tape.frames[0].pads[0].buttons, BUTTON_A);

    let mut reactive = plan;
    reactive.cancellation_conditions = vec![crate::option_execution::OptionCondition::TargetLost {
        target: "crawl-entry".into(),
    }];
    let candidate = Candidate {
        actions: vec![MacroAction::GameTactic { plan: reactive }],
        ..candidate
    };
    assert!(matches!(
        candidate.compile(),
        Err(SearchError::NonCanonicalTape(_))
    ));
}

#[test]
fn exact_motion_path_is_a_first_class_static_search_macro() {
    use crate::motion_path::{SamplePhase, StickPath, StickPoint};
    let plan = MotionPathPlan {
        schema: crate::motion_path::MOTION_PATH_SCHEMA_V1.into(),
        path: StickPath::Bezier {
            control: [
                StickPoint { x: 0, y: 0 },
                StickPoint { x: 0, y: 8 },
                StickPoint { x: 8, y: 8 },
                StickPoint { x: 8, y: 0 },
            ],
        },
        duration_ticks: 2,
        sample_phase: SamplePhase {
            numerator: 1,
            denominator: 1,
        },
        cancellation_conditions: Vec::new(),
    };
    let candidate = Candidate {
        schema: CANDIDATE_SCHEMA.into(),
        segment: SegmentProfile::Fsp103ToFsp104,
        boot: TapeBoot::Process,
        actions: vec![MacroAction::MotionPath { plan }],
        ancestry: Ancestry::default(),
    };
    let tape = candidate.compile().unwrap();
    assert_eq!(tape.frames.len(), 2);
    assert_eq!(
        (
            tape.frames[0].pads[0].stick_x,
            tape.frames[0].pads[0].stick_y
        ),
        (4, 6)
    );
    assert_eq!(
        (
            tape.frames[1].pads[0].stick_x,
            tape.frames[1].pads[0].stick_y
        ),
        (8, 0)
    );
}

#[test]
fn absolute_tape_inference_keeps_route_analog_but_boot_rejects_it() {
    let source = Candidate::baseline(SegmentProfile::BootToFsp103)
        .compile()
        .unwrap();
    let imported = Candidate::from_absolute_tape(SegmentProfile::BootToFsp103, &source).unwrap();
    assert_eq!(imported.compile().unwrap(), source);
    assert!(
        imported
            .actions
            .iter()
            .any(|action| matches!(action, MacroAction::Press { .. }))
    );

    let mut analog = Candidate::baseline(SegmentProfile::Fsp103ToFsp104)
        .compile()
        .unwrap();
    let disconnected = RawPadState {
        connected: false,
        error: -1,
        ..RawPadState::default()
    };
    for frame in &mut analog.frames {
        frame.owned_ports = 0x01;
        frame.pads[1..].fill(disconnected);
    }
    let imported_route =
        Candidate::from_absolute_tape(SegmentProfile::Fsp103ToFsp104, &analog).unwrap();
    assert_eq!(imported_route.compile().unwrap(), analog);
    assert!(matches!(
        Candidate::from_absolute_tape(SegmentProfile::BootToFsp103, &analog),
        Err(SearchError::NonCanonicalTape(_))
    ));

    let long_hold = Candidate {
        schema: CANDIDATE_SCHEMA.into(),
        segment: SegmentProfile::BootToFsp103,
        boot: TapeBoot::Process,
        actions: vec![
            MacroAction::Press {
                buttons: vec![ControllerButton::A],
                hold_frames: 30,
                neutral_frames: 0,
            },
            MacroAction::Press {
                buttons: vec![ControllerButton::A],
                hold_frames: 30,
                neutral_frames: 1,
            },
        ],
        ancestry: Ancestry::default(),
    }
    .compile()
    .unwrap();
    assert_eq!(
        Candidate::from_absolute_tape(SegmentProfile::BootToFsp103, &long_hold)
            .unwrap()
            .compile()
            .unwrap(),
        long_hold
    );
}

#[test]
fn promoted_tunnel_suffix_imports_losslessly_as_compact_pad_runs() {
    let disconnected = RawPadState {
        connected: false,
        error: -1,
        ..RawPadState::default()
    };
    let tape = InputTape {
        frames: (0..421)
            .map(|index| {
                let mut frame = InputFrame {
                    owned_ports: 1,
                    pads: [disconnected; 4],
                    ..InputFrame::default()
                };
                frame.pads[0] = RawPadState::default();
                frame.pads[0].stick_y = if index < 200 {
                    96
                } else if index < 400 {
                    127
                } else {
                    0
                };
                frame
            })
            .collect(),
        ..InputTape::default()
    };
    assert_eq!(tape.frames.len(), 421);
    let candidate =
        Candidate::from_absolute_tape(SegmentProfile::LinkControlToTunnelCrawlStart, &tape)
            .unwrap();
    assert_eq!(candidate.compile().unwrap(), tape);
    assert!(candidate.actions.len() < tape.frames.len());
    assert!(
        candidate
            .actions
            .iter()
            .all(|action| matches!(action, MacroAction::PadRun { .. }))
    );
    assert!(
        Candidate::baseline(SegmentProfile::LinkControlToTunnelCrawlStart)
            .validate()
            .is_err()
    );

    let mut rng = SplitMix64::new(0x7a5_2026);
    let mut stick_mutations = 0;
    let mut button_mutations = 0;
    for _ in 0..256 {
        let child = mutate(&candidate, 1, &mut rng).unwrap();
        let mutation = child.ancestry.mutation.as_deref().unwrap();
        stick_mutations += usize::from(mutation.starts_with("pad_stick["));
        button_mutations += usize::from(mutation.starts_with("pad_toggle_b["));
    }
    assert!(stick_mutations > 20);
    assert!(button_mutations > 20);
}

#[test]
fn boot_mutation_directly_targets_press_gaps() {
    let parent = Candidate::baseline(SegmentProfile::BootToFsp103);
    let mut rng = SplitMix64::new(0x5eed);
    let mut gap_mutations = 0;
    let mut shrink_mutations = 0;
    for _ in 0..256 {
        let child = mutate(&parent, 0, &mut rng).unwrap();
        let mutation = child.ancestry.mutation.as_deref().unwrap();
        if mutation.starts_with("boot_gap[") || mutation.starts_with("boot_shrink[") {
            let changed: Vec<_> = parent
                .actions
                .iter()
                .zip(&child.actions)
                .enumerate()
                .filter(|(_, (before, after))| before != after)
                .collect();
            assert!(changed.len() <= 1);
            if let Some((_, (_, action))) = changed.first() {
                assert!(matches!(action, MacroAction::Press { .. }));
            }
        }
        gap_mutations += usize::from(mutation.starts_with("boot_gap["));
        shrink_mutations += usize::from(mutation.starts_with("boot_shrink["));
    }
    assert!(gap_mutations > 50);
    assert!(shrink_mutations > 50);
}

#[test]
fn score_is_depth_then_tick_and_fractional_repeats_are_invalid() {
    let score = |depth, successes, attempts, ticks| {
        CandidateResult {
            goal_reached: Some(depth == 4),
            milestone_depth: depth,
            attempts,
            successes,
            first_hit_ticks: ticks,
            risk_events: None,
            boundary_compatibility: BoundaryCompatibility::Unknown,
        }
        .score()
        .unwrap()
    };
    assert!(score(4, 10, 10, vec![500; 10]) > score(3, 10, 10, vec![1; 10]));
    assert!(score(4, 10, 10, vec![99; 10]) > score(4, 10, 10, vec![100; 10]));
    let feasible = CandidateResult {
        goal_reached: Some(true),
        milestone_depth: 1,
        attempts: 1,
        successes: 1,
        first_hit_ticks: vec![10_000],
        risk_events: None,
        boundary_compatibility: BoundaryCompatibility::Unknown,
    }
    .score()
    .unwrap();
    let infeasible = CandidateResult {
        goal_reached: Some(false),
        milestone_depth: u16::MAX,
        attempts: 1,
        successes: 1,
        first_hit_ticks: vec![1],
        risk_events: None,
        boundary_compatibility: BoundaryCompatibility::Unknown,
    }
    .score()
    .unwrap();
    assert!(feasible > infeasible);
    assert!(matches!(
        CandidateResult {
            goal_reached: Some(true),
            milestone_depth: 4,
            attempts: 10,
            successes: 9,
            first_hit_ticks: vec![500; 9],
            risk_events: None,
            boundary_compatibility: BoundaryCompatibility::Unknown,
        }
        .score(),
        Err(SearchError::InvalidResult)
    ));
}

#[test]
fn lexicographic_score_uses_every_declared_axis_in_order() {
    let score = LexicographicScore {
        goal_feasible: true,
        milestone_depth: 4,
        successes: 3,
        attempts: 3,
        median_first_hit_tick: 100,
        best_first_hit_tick: 100,
        tape_frames: 120,
        input_complexity: 12,
        risk_events: Some(2),
        boundary_compatibility: BoundaryCompatibility::Compatible,
    };

    assert!(
        score
            > LexicographicScore {
                goal_feasible: false,
                milestone_depth: u16::MAX,
                median_first_hit_tick: 0,
                best_first_hit_tick: 0,
                tape_frames: 0,
                input_complexity: 0,
                risk_events: Some(0),
                boundary_compatibility: BoundaryCompatibility::Exact,
                ..score
            }
    );
    assert!(
        score
            > LexicographicScore {
                milestone_depth: 3,
                median_first_hit_tick: 0,
                best_first_hit_tick: 0,
                tape_frames: 0,
                input_complexity: 0,
                risk_events: Some(0),
                boundary_compatibility: BoundaryCompatibility::Exact,
                ..score
            }
    );
    assert!(
        score
            > LexicographicScore {
                median_first_hit_tick: 101,
                best_first_hit_tick: 0,
                tape_frames: 0,
                input_complexity: 0,
                risk_events: Some(0),
                boundary_compatibility: BoundaryCompatibility::Exact,
                ..score
            }
    );
    assert!(
        score
            > LexicographicScore {
                tape_frames: 121,
                input_complexity: 0,
                risk_events: Some(0),
                boundary_compatibility: BoundaryCompatibility::Exact,
                ..score
            }
    );
    assert!(
        score
            > LexicographicScore {
                input_complexity: 13,
                risk_events: Some(0),
                boundary_compatibility: BoundaryCompatibility::Exact,
                ..score
            }
    );
    assert!(
        score
            > LexicographicScore {
                risk_events: Some(3),
                boundary_compatibility: BoundaryCompatibility::Exact,
                ..score
            }
    );
    assert!(
        score
            > LexicographicScore {
                risk_events: None,
                boundary_compatibility: BoundaryCompatibility::Exact,
                ..score
            }
    );
    assert!(
        score
            > LexicographicScore {
                boundary_compatibility: BoundaryCompatibility::Unknown,
                ..score
            }
    );
    assert!(
        LexicographicScore {
            boundary_compatibility: BoundaryCompatibility::Exact,
            ..score
        } > score
    );
    assert!(
        LexicographicScore {
            boundary_compatibility: BoundaryCompatibility::Unknown,
            ..score
        } > LexicographicScore {
            boundary_compatibility: BoundaryCompatibility::Incompatible,
            ..score
        }
    );
}

#[test]
fn input_complexity_counts_native_transitions_after_compilation() {
    let mut first = InputFrame {
        owned_ports: 0b0101,
        wait_condition: crate::tape::WaitCondition::NameEntryActive,
        wait_timeout_ticks: 20,
        ..InputFrame::default()
    };
    first.pads[0].buttons = 0b0101;
    first.pads[0].stick_x = 1;
    first.pads[0].analog_a = 2;
    first.pads[1].connected = false;
    first.pads[1].error = -1;

    let mut second = first.clone();
    second.owned_ports = 0b0001;
    second.pads[0].buttons = 0b0110;
    second.pads[0].trigger_left = 1;
    let tape = InputTape {
        frames: vec![first, second.clone(), second],
        ..InputTape::default()
    };
    assert_eq!(tape_input_complexity(&tape), 14);
    let decoded = InputTape::decode(&tape.encode().unwrap()).unwrap().tape;
    assert_eq!(tape_input_complexity(&decoded), 14);

    let one_run = Candidate {
        schema: CANDIDATE_SCHEMA.into(),
        segment: SegmentProfile::BootToFsp103,
        boot: TapeBoot::Process,
        actions: vec![MacroAction::Move {
            angle_degrees: 0,
            magnitude: 127,
            frames: 2,
        }],
        ancestry: Ancestry::default(),
    };
    let split_run = Candidate {
        actions: vec![
            MacroAction::Move {
                angle_degrees: 0,
                magnitude: 127,
                frames: 1,
            },
            MacroAction::Move {
                angle_degrees: 0,
                magnitude: 127,
                frames: 1,
            },
        ],
        ..one_run.clone()
    };
    let compiled = one_run.compile().unwrap();
    let split_compiled = split_run.compile().unwrap();
    assert_eq!(compiled, split_compiled);
    assert_eq!(
        tape_input_complexity(&compiled),
        tape_input_complexity(&split_compiled)
    );
}

#[test]
fn population_v3_requires_complexity_while_legacy_populations_remain_readable() {
    let candidate = Candidate::baseline(SegmentProfile::BootToFsp103);
    let member = PopulationMember {
        candidate_id: candidate.id().unwrap(),
        candidate_file: PathBuf::from("candidate.json"),
        tape_file: PathBuf::from("candidate.tape"),
        frame_count: candidate.frame_count(),
        input_complexity: None,
        ancestry: Ancestry::default(),
    };
    let manifest = PopulationManifest {
        schema: POPULATION_SCHEMA.into(),
        segment: candidate.segment,
        boot: candidate.boot,
        generation: 0,
        rng_seed: 1,
        members: vec![member],
    };
    assert!(matches!(
        validate_population_schema(&manifest),
        Err(SearchError::InvalidPopulation)
    ));

    let legacy_v2 = PopulationManifest {
        schema: LEGACY_POPULATION_SCHEMA_V2.into(),
        ..manifest.clone()
    };
    assert!(validate_population_schema(&legacy_v2).is_ok());
    let legacy_v1 = PopulationManifest {
        schema: LEGACY_POPULATION_SCHEMA_V1.into(),
        ..manifest
    };
    assert!(validate_population_schema(&legacy_v1).is_ok());
}

#[test]
fn current_results_require_an_explicit_consistent_goal_verdict() {
    let candidate = Candidate::baseline(SegmentProfile::BootToFsp103);
    let candidate_id = candidate.id().unwrap();
    let manifest = PopulationManifest {
        schema: POPULATION_SCHEMA.into(),
        segment: candidate.segment,
        boot: candidate.boot.clone(),
        generation: 0,
        rng_seed: 1,
        members: vec![PopulationMember {
            candidate_id: candidate_id.clone(),
            candidate_file: PathBuf::from("candidate.json"),
            tape_file: PathBuf::from("candidate.tape"),
            frame_count: candidate.frame_count(),
            input_complexity: Some(0),
            ancestry: Ancestry::default(),
        }],
    };
    let result = |goal_reached, depth| SearchResults {
        schema: RESULTS_SCHEMA.into(),
        segment: manifest.segment,
        boot: manifest.boot.clone(),
        candidates: BTreeMap::from([(
            candidate_id.clone(),
            CandidateResult {
                goal_reached,
                milestone_depth: depth,
                attempts: 1,
                successes: u32::from(depth != 0),
                first_hit_ticks: (depth != 0).then_some(10).into_iter().collect(),
                risk_events: None,
                boundary_compatibility: BoundaryCompatibility::Unknown,
            },
        )]),
    };
    assert!(matches!(
        rank_population(&manifest, &result(None, 2)),
        Err(SearchError::InvalidResult)
    ));
    assert!(matches!(
        rank_population(&manifest, &result(Some(true), 0)),
        Err(SearchError::InvalidResult)
    ));
    assert!(rank_population(&manifest, &result(Some(true), 2)).is_ok());
}

#[test]
fn evaluator_trials_reject_disagreement() {
    let candidate = Candidate::baseline(SegmentProfile::Fsp103ToFsp104);
    let candidate_id = candidate.id().unwrap();
    let manifest = PopulationManifest {
        schema: POPULATION_SCHEMA.into(),
        segment: candidate.segment,
        boot: candidate.boot.clone(),
        generation: 0,
        rng_seed: 1,
        members: vec![PopulationMember {
            candidate_id: candidate_id.clone(),
            candidate_file: PathBuf::from("candidate.json"),
            tape_file: PathBuf::from("candidate.tape"),
            frame_count: candidate.frame_count(),
            input_complexity: Some(0),
            ancestry: Ancestry::default(),
        }],
    };
    let artifact = |depth, tick| EvaluationArtifact {
        schema_version: 1,
        candidate_id: candidate_id.clone(),
        search_result: CandidateResult {
            goal_reached: None,
            milestone_depth: depth,
            attempts: 1,
            successes: 1,
            first_hit_ticks: vec![tick],
            risk_events: None,
            boundary_compatibility: BoundaryCompatibility::Unknown,
        },
    };
    assert!(matches!(
        collect_results(&manifest, [artifact(3, 570), artifact(4, 603)]),
        Err(SearchError::InvalidResult)
    ));
}

#[test]
fn population_results_and_leaderboard_are_partitioned_by_boot_origin() {
    let root = std::env::temp_dir().join(format!(
        "huntctl-search-boot-partition-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let stage = Candidate::baseline(SegmentProfile::Fsp103ToFsp104);
    let manifest = write_explicit_population(&root, stage.segment, 0, vec![stage.clone()]).unwrap();
    assert_eq!(manifest.boot, stage.boot);

    let candidate_id = manifest.members[0].candidate_id.clone();
    let results = SearchResults {
        schema: RESULTS_SCHEMA.into(),
        segment: manifest.segment,
        boot: manifest.boot.clone(),
        candidates: BTreeMap::from([(
            candidate_id.clone(),
            CandidateResult {
                goal_reached: Some(false),
                milestone_depth: 1,
                attempts: 2,
                successes: 2,
                first_hit_ticks: vec![42, 42],
                risk_events: None,
                boundary_compatibility: BoundaryCompatibility::Unknown,
            },
        )]),
    };
    let leaderboard = rank_population(&manifest, &results).unwrap();
    assert_eq!(leaderboard[0].boot, manifest.boot);

    let process_results = SearchResults {
        boot: TapeBoot::Process,
        ..results.clone()
    };
    assert!(matches!(
        rank_population(&manifest, &process_results),
        Err(SearchError::BootMismatch)
    ));

    let mut process = stage.clone();
    process.boot = TapeBoot::Process;
    assert!(matches!(
        write_explicit_population(&root.join("mixed"), stage.segment, 0, vec![stage, process],),
        Err(SearchError::BootMismatch)
    ));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn seeded_evolution_is_reproducible_and_keeps_champion() {
    let root = std::env::temp_dir().join(format!("huntctl-search-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let first = write_seed_population(
        &root.join("g0"),
        Candidate::baseline(SegmentProfile::Fsp103ToFsp104),
        8,
        42,
    )
    .unwrap();
    let champion = first.members[3].candidate_id.clone();
    let candidates = first
        .members
        .iter()
        .enumerate()
        .map(|(index, member)| {
            (
                member.candidate_id.clone(),
                CandidateResult {
                    goal_reached: Some(index == 3),
                    milestone_depth: if index == 3 { 4 } else { 3 },
                    attempts: 2,
                    successes: 2,
                    first_hit_ticks: vec![100 + index as u64; 2],
                    risk_events: None,
                    boundary_compatibility: BoundaryCompatibility::Unknown,
                },
            )
        })
        .collect();
    let results = SearchResults {
        schema: RESULTS_SCHEMA.into(),
        segment: first.segment,
        boot: first.boot.clone(),
        candidates,
    };
    let config = EvolutionConfig {
        population_size: 8,
        elite_count: 2,
        rng_seed: 99,
    };
    let next = evolve_population(
        &root.join("g0/manifest.json"),
        &results,
        &root.join("g1"),
        config,
    )
    .unwrap();
    let again = evolve_population(
        &root.join("g0/manifest.json"),
        &results,
        &root.join("g1-again"),
        config,
    )
    .unwrap();
    assert_eq!(next.members[0].candidate_id, champion);
    assert_eq!(
        next.members
            .iter()
            .map(|member| &member.candidate_id)
            .collect::<Vec<_>>(),
        again
            .members
            .iter()
            .map(|member| &member.candidate_id)
            .collect::<Vec<_>>()
    );
    assert!(
        next.members.iter().skip(2).all(|member| {
            member.ancestry.generation == 1 && member.ancestry.parent_id.is_some()
        })
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn evolution_retains_an_exact_archived_candidate_before_new_proposals() {
    let root = std::env::temp_dir().join(format!("huntctl-search-retained-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let first = write_seed_population(
        &root.join("g0"),
        Candidate::baseline(SegmentProfile::Fsp103ToFsp104),
        6,
        121,
    )
    .unwrap();
    let retained_member = &first.members[5];
    let retained: Candidate = serde_json::from_slice(
        &fs::read(root.join("g0").join(&retained_member.candidate_file)).unwrap(),
    )
    .unwrap();
    let results = SearchResults {
        schema: RESULTS_SCHEMA.into(),
        segment: first.segment,
        boot: first.boot.clone(),
        candidates: first
            .members
            .iter()
            .enumerate()
            .map(|(index, member)| {
                (
                    member.candidate_id.clone(),
                    CandidateResult {
                        goal_reached: Some(true),
                        milestone_depth: 2,
                        attempts: 1,
                        successes: 1,
                        first_hit_ticks: vec![100 + index as u64],
                        risk_events: None,
                        boundary_compatibility: BoundaryCompatibility::Unknown,
                    },
                )
            })
            .collect(),
    };
    let next = evolve_population_with_retained_and_proposals(
        &root.join("g0/manifest.json"),
        &results,
        &root.join("g1"),
        EvolutionConfig {
            population_size: 6,
            elite_count: 1,
            rng_seed: 122,
        },
        std::slice::from_ref(&retained),
        &[],
    )
    .unwrap();
    assert_eq!(next.members[1].candidate_id, retained.id().unwrap());
    assert_eq!(next.members[1].ancestry, retained.ancestry);
    fs::remove_dir_all(root).unwrap();
}
