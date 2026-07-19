use super::*;
use crate::search::write_explicit_population;
use std::time::{SystemTime, UNIX_EPOCH};

fn heldout_score(first_hit_tick: u64) -> LexicographicScore {
    LexicographicScore {
        goal_feasible: true,
        milestone_depth: 2,
        successes: 2,
        attempts: 2,
        median_first_hit_tick: first_hit_tick,
        best_first_hit_tick: first_hit_tick,
        tape_frames: 8,
        input_complexity: 2,
        risk_events: Some(0),
        boundary_compatibility: crate::search::BoundaryCompatibility::Exact,
    }
}

#[test]
fn proposal_readiness_requires_observed_supported_native_terminals() {
    assert!(!native_terminals_support_required_facts([]));
    assert!(native_terminals_support_required_facts([
        Some(HarnessTerminalReason::Reached),
        Some(HarnessTerminalReason::Exhausted),
    ]));
    assert!(!native_terminals_support_required_facts([None]));
    assert!(!native_terminals_support_required_facts([Some(
        HarnessTerminalReason::Unsupported,
    )]));
    assert!(!native_terminals_support_required_facts([Some(
        HarnessTerminalReason::CapabilityMismatch,
    )]));
}

#[test]
fn learned_holdout_must_match_or_beat_a_native_baseline() {
    assert!(!learned_holdout_scores_adequate([(
        false,
        heldout_score(10)
    ),]));
    assert!(!learned_holdout_scores_adequate([(
        true,
        heldout_score(10)
    ),]));
    assert!(learned_holdout_scores_adequate([
        (false, heldout_score(10)),
        (true, heldout_score(10)),
    ]));
    assert!(learned_holdout_scores_adequate([
        (false, heldout_score(10)),
        (true, heldout_score(5)),
    ]));
    assert!(!learned_holdout_scores_adequate([
        (false, heldout_score(10)),
        (true, heldout_score(20)),
    ]));
}

#[test]
fn attempt_outcomes_keep_all_terminal_classes_distinct() {
    let class = |timed_out, cancelled, trace, infrastructure, goal| {
        classify_outcome(timed_out, cancelled, trace, infrastructure, goal).class
    };
    assert_eq!(
        class(false, false, None, None, true),
        EpisodeOutcomeClass::Successful
    );
    assert_eq!(
        class(false, false, None, None, false),
        EpisodeOutcomeClass::Failed
    );
    assert_eq!(
        class(
            false,
            false,
            None,
            Some("invalid native milestone result: worker exit None disagrees"),
            false,
        ),
        EpisodeOutcomeClass::Crashed
    );
    assert_eq!(
        class(true, false, None, None, false),
        EpisodeOutcomeClass::TimedOut
    );
    assert_eq!(
        class(
            false,
            false,
            None,
            Some("invalid native milestone result: bad boundary digest"),
            false,
        ),
        EpisodeOutcomeClass::Desynced
    );
    assert_eq!(
        class(
            false,
            false,
            None,
            Some("could not launch Dusklight: unsupported executable"),
            false,
        ),
        EpisodeOutcomeClass::Unsupported
    );
    assert_eq!(
        class(
            false,
            false,
            Some("gameplay trace capacity was exhausted"),
            None,
            false,
        ),
        EpisodeOutcomeClass::Truncated
    );
}

#[test]
fn anchored_parser_requires_exact_program_source_and_crawl_evidence() {
    assert!(validate_anchored_game_args(&["--stage".into(), "F_SP103,1,1,3".into()]).is_err());
    assert!(validate_anchored_game_args(&["--stage=F_SP103,1,1,3".into()]).is_err());
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("huntctl-anchored-proof-{unique}"));
    fs::create_dir_all(&root).unwrap();
    let prefix_path = root.join("prefix.tape");
    let prefix = InputTape {
        tick_rate_numerator: 30,
        tick_rate_denominator: 1,
        frames: vec![crate::tape::InputFrame::default(); 2],
        ..InputTape::default()
    };
    fs::write(&prefix_path, prefix.encode().unwrap()).unwrap();
    let program = crate::milestone_dsl::compile_source(
        r#"milestones 1.0
milestone link_control {
  phase post_sim
  when stage.name == "F_SP103"
}
milestone near_tunnel {
  phase post_sim
  when stage.name == "F_SP104"
}
milestone tunnel_crawl_start {
  phase post_sim
  when stage.name == "F_SP104" && stage.room == 1 && stage.spawn == 0 && player.procedure == "crawl_start"
}
"#,
    )
    .unwrap();
    let program_path = root.join("objective.dmsp");
    fs::write(&program_path, &program.bytes).unwrap();
    let game_path = root.join("game.exe");
    let dvd_path = root.join("disc.iso");
    fs::write(&game_path, b"game-build").unwrap();
    fs::write(&dvd_path, b"disc-build").unwrap();
    let prepared = prepare_anchored_objective(
        &AnchoredObjectiveConfig {
            segment: SegmentProfile::LinkControlToTunnelCrawlStart,
            prefix_tape: prefix_path,
            milestone_program: program_path,
            game: game_path,
            dvd: dvd_path,
            source_milestone: "link_control".into(),
            source_boundary_fingerprint: "a".repeat(32),
            goal_milestone: "tunnel_crawl_start".into(),
        },
        root.join("runtime.dmsp"),
    )
    .unwrap();
    let fingerprint = |digest: String| {
        serde_json::json!({
            "schema": "dusklight.milestone-boundary/v1",
            "algorithm": "xxh3-128",
            "canonical_encoding": "little-endian-fixed-v1",
            "digest": digest,
        })
    };
    let authored = |id: &str,
                    definition: &AuthoredDefinitionExpectation,
                    sim_tick: u64,
                    tape_frame: u64,
                    boundary_index: u64,
                    digest: String,
                    goal: bool| {
        serde_json::json!({
            "id": id,
            "hit": true,
            "phase": definition.phase,
            "stable_ticks": definition.stable_ticks,
            "definition_digest": definition.digest,
            "program_digest": prepared.identity.milestone_program_sha256,
            "boundary_index": boundary_index,
            "sim_tick": sim_tick,
            "tape_frame": tape_frame,
            "evidence": {
                "stage": {
                    "name": if goal { "F_SP104" } else { "F_SP103" },
                    "room": 1,
                    "point": if goal { 0 } else { 1 },
                },
                "player": {
                    "present": true,
                    "is_link": true,
                    "procedure_id": if goal { 53 } else { 3 },
                },
                "boundary_fingerprint": fingerprint(digest),
            }
        })
    };
    let result = serde_json::json!({
        "schema": {"name": "dusklight.automation.milestones", "version": 1},
        "goal": "tunnel_crawl_start",
        "goal_reached": true,
        "program_digest": prepared.identity.milestone_program_sha256,
        "milestones": [
            authored("link_control", &prepared.source, 1, 1, 2, "a".repeat(32), false),
            authored("near_tunnel", &prepared.progress[0], 2, 2, 3, "b".repeat(32), false),
            authored("tunnel_crawl_start", &prepared.goal, 3, 3, 4, "c".repeat(32), true),
        ],
    });
    let result_path = root.join("result.json");
    fs::write(&result_path, serde_json::to_vec_pretty(&result).unwrap()).unwrap();
    let score = parse_anchored_milestones(&result_path, &prepared, &TapeBoot::Process).unwrap();
    assert!(score.goal_reached);
    assert_eq!(score.depth, 3);
    assert_eq!(score.score_tick, Some(1));

    let clear_hit_evidence = |milestone: &mut serde_json::Value| {
        let object = milestone.as_object_mut().unwrap();
        object.insert("hit".into(), false.into());
        for field in [
            "boundary_index",
            "sim_tick",
            "tape_frame",
            "evidence",
            "projections",
        ] {
            object.remove(field);
        }
    };
    let mut near_miss = result.clone();
    near_miss["goal_reached"] = false.into();
    clear_hit_evidence(&mut near_miss["milestones"][2]);
    fs::write(&result_path, serde_json::to_vec_pretty(&near_miss).unwrap()).unwrap();
    let near_score =
        parse_anchored_milestones(&result_path, &prepared, &TapeBoot::Process).unwrap();
    assert!(!near_score.goal_reached);
    assert_eq!(near_score.depth, 2);
    assert_eq!(near_score.deepest, "near_tunnel");

    let mut ordinary_failure = near_miss;
    clear_hit_evidence(&mut ordinary_failure["milestones"][1]);
    fs::write(
        &result_path,
        serde_json::to_vec_pretty(&ordinary_failure).unwrap(),
    )
    .unwrap();
    let ordinary_score =
        parse_anchored_milestones(&result_path, &prepared, &TapeBoot::Process).unwrap();
    assert!(!ordinary_score.goal_reached);
    assert_eq!(ordinary_score.depth, 1);
    assert_eq!(ordinary_score.deepest, "link_control");

    let suffix_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../routes/Glitch Exhibition/intro/segments/human420.tape");
    let suffix = InputTape::decode(&fs::read(suffix_path).unwrap())
        .unwrap()
        .tape;
    let candidate =
        Candidate::from_absolute_tape(SegmentProfile::LinkControlToTunnelCrawlStart, &suffix)
            .unwrap();
    let population_root = root.join("population");
    let manifest = write_explicit_population(
        &population_root,
        SegmentProfile::LinkControlToTunnelCrawlStart,
        0,
        vec![candidate],
    )
    .unwrap();
    let trials = build_anchored_trials(
        &manifest,
        &fs::canonicalize(&population_root).unwrap(),
        &root.join("attempts"),
        1,
        &prepared,
    )
    .unwrap();
    let full = InputTape::decode(&fs::read(&trials[0].tape).unwrap())
        .unwrap()
        .tape;
    assert_eq!(full.frames.len(), prefix.frames.len() + suffix.frames.len());
    assert_eq!(
        &full.frames[..prefix.frames.len()],
        prefix.frames.as_slice()
    );
    assert_eq!(
        &full.frames[prefix.frames.len()..],
        suffix.frames.as_slice()
    );
    bind_population_objective(&population_root, &prepared.identity).unwrap();
    bind_population_objective(&population_root, &prepared.identity).unwrap();
    let mut different_objective = prepared.identity.clone();
    different_objective.digest = "d".repeat(64);
    assert!(bind_population_objective(&population_root, &different_objective).is_err());

    let member_tape = population_root.join(&manifest.members[0].tape_file);
    let mut tampered = suffix.clone();
    tampered.frames[0].pads[0].buttons ^= 0x0100;
    fs::write(&member_tape, tampered.encode().unwrap()).unwrap();
    assert!(
        build_anchored_trials(
            &manifest,
            &fs::canonicalize(&population_root).unwrap(),
            &root.join("tampered-attempts"),
            1,
            &prepared,
        )
        .is_err()
    );

    let mut wrong = result;
    wrong["milestones"][0]["evidence"]["boundary_fingerprint"]["digest"] =
        serde_json::Value::String("c".repeat(32));
    fs::write(&result_path, serde_json::to_vec_pretty(&wrong).unwrap()).unwrap();
    assert!(parse_anchored_milestones(&result_path, &prepared, &TapeBoot::Process).is_err());

    let mut mismatched_objective = prepared.config.clone();
    mismatched_objective.goal_milestone = "different_goal".into();
    let mismatch = evaluate_prepared_anchored_population(
        &AnchoredEvaluateConfig {
            evaluation: EvaluateConfig {
                population_path: population_root.join("manifest.json"),
                game: prepared.config.game.clone(),
                dvd: prepared.config.dvd.clone(),
                output_root: root.join("mismatch-evidence"),
                episode_store: None,
                results_path: root.join("mismatch-results.json"),
                working_directory: root.clone(),
                game_args_prefix: Vec::new(),
                workers: 1,
                repetitions: 2,
                timeout: Duration::from_secs(1),
                harness: None,
            },
            objective: mismatched_objective,
        },
        &prepared,
    );
    assert!(matches!(mismatch, Err(EvaluateError::InvalidConfig(_))));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn named_value_parity_is_equal_different_or_incomparable_without_topology() {
    let projection = |identity: &str, digest: &str, available: bool| ValueProjectionEvidence {
        name: "handoff-state".into(),
        identity: identity.into(),
        available,
        value_fingerprint: available.then(|| BoundaryFingerprint {
            schema: "dusklight.value-projection/v1".into(),
            algorithm: "xxh3-128".into(),
            canonical_encoding: "little-endian-exact-v1".into(),
            digest: digest.into(),
        }),
        values: vec![serde_json::json!({"kind":"flag", "available":available})],
    };
    let reference = projection(&"a".repeat(64), &"1".repeat(32), true);
    assert_eq!(
        compare_value_projections(
            &reference,
            &projection(&"a".repeat(64), &"1".repeat(32), true)
        ),
        ValueParityComparison::Equal
    );
    assert_eq!(
        compare_value_projections(
            &reference,
            &projection(&"a".repeat(64), &"2".repeat(32), true)
        ),
        ValueParityComparison::Different
    );
    assert_eq!(
        compare_value_projections(
            &reference,
            &projection(&"b".repeat(64), &"1".repeat(32), true)
        ),
        ValueParityComparison::Incomparable
    );
    assert_eq!(
        compare_value_projections(
            &reference,
            &projection(&"a".repeat(64), &"1".repeat(32), false)
        ),
        ValueParityComparison::Incomparable
    );
}

#[test]
fn archive_context_partitions_named_rng_actors_and_downstream_boundaries() {
    let boundary = |digest: &str| BoundaryFingerprint {
        schema: "dusklight.milestone-boundary/v4".into(),
        algorithm: "xxh3-128".into(),
        canonical_encoding: "little-endian-fixed-v4".into(),
        digest: digest.into(),
    };
    let projection = |digest: &str| ValueProjectionEvidence {
        name: "handoff-state".into(),
        identity: "a".repeat(64),
        available: true,
        value_fingerprint: Some(BoundaryFingerprint {
            schema: "dusklight.value-projection/v1".into(),
            algorithm: "xxh3-128".into(),
            canonical_encoding: "little-endian-exact-v1".into(),
            digest: digest.into(),
        }),
        values: vec![
            serde_json::json!({"kind":"rng", "available":true}),
            serde_json::json!({"kind":"actor_population", "available":true}),
        ],
    };
    let boundaries = BTreeMap::from([("goal".into(), boundary(&"1".repeat(32)))]);
    let projections = |digest: &str| {
        BTreeMap::from([(
            "goal".into(),
            BTreeMap::from([("handoff-state".into(), projection(digest))]),
        )])
    };
    let reference = archive_behavior_context_from_evidence(
        &boundaries,
        &projections(&"2".repeat(32)),
        Some("c".repeat(64)),
    );
    assert!(reference.objective_rng_identity.is_some());
    assert!(reference.actor_population_identity.is_some());
    assert_eq!(reference.contact_behavior_identity, Some("c".repeat(64)));
    assert!(reference.boundary_state_identity.is_some());
    assert!(reference.downstream_state_identity.is_some());

    let changed_projection = archive_behavior_context_from_evidence(
        &boundaries,
        &projections(&"3".repeat(32)),
        Some("d".repeat(64)),
    );
    assert_ne!(
        reference.objective_rng_identity,
        changed_projection.objective_rng_identity
    );
    assert_ne!(
        reference.actor_population_identity,
        changed_projection.actor_population_identity
    );
    assert_ne!(
        reference.downstream_state_identity,
        changed_projection.downstream_state_identity
    );
    assert_ne!(
        reference.contact_behavior_identity,
        changed_projection.contact_behavior_identity
    );

    let changed_boundary = BTreeMap::from([("goal".into(), boundary(&"4".repeat(32)))]);
    let changed_downstream = archive_behavior_context_from_evidence(
        &changed_boundary,
        &projections(&"2".repeat(32)),
        Some("c".repeat(64)),
    );
    assert_eq!(
        reference.objective_rng_identity,
        changed_downstream.objective_rng_identity
    );
    assert_eq!(
        reference.actor_population_identity,
        changed_downstream.actor_population_identity
    );
    assert_ne!(
        reference.boundary_state_identity,
        changed_downstream.boundary_state_identity
    );
    assert_ne!(
        reference.downstream_state_identity,
        changed_downstream.downstream_state_identity
    );
}

#[test]
fn contact_behavior_identity_is_portable_and_run_deduplicated() {
    fn contact_trace(flags: u32, owner: u32, records: usize) -> crate::trace::DecodedTrace {
        let collision = crate::trace::TracePlayerBackgroundCollision {
            flags,
            ground_height: 12.0,
            roof_height: 100.0,
            water_height: -100.0,
            ground_bg_index: Some(2),
            ground_poly_index: Some(3),
            ground_owner_session_process_id: Some(owner),
            ground_plane: [0.0, 1.0, 0.0, -12.0],
            ground_identity_present: true,
            roof_bg_index: None,
            roof_poly_index: None,
            roof_owner_session_process_id: None,
            roof_identity_present: false,
            water_bg_index: None,
            water_poly_index: None,
            water_owner_session_process_id: None,
            water_identity_present: false,
            walls: std::array::from_fn(|_| crate::trace::TraceCollisionWall {
                identity_present: false,
                bg_index: None,
                poly_index: None,
                owner_session_process_id: Some(owner),
                angle_y: 0,
                flags: 0,
            }),
            old_position: [1.0, 2.0, 3.0],
            resolved_frame_displacement: [1.0, 0.0, 0.0],
            final_position: [2.0, 2.0, 3.0],
        };
        crate::trace::DecodedTrace {
            version: 5,
            boot: TapeBoot::Process,
            tick_rate_numerator: 30,
            tick_rate_denominator: 1,
            requested_channels: 0,
            capacity_exhausted: false,
            retention: None,
            channel_formats: BTreeMap::new(),
            records: (0..records)
                .map(|_| crate::trace::TraceRecord {
                    player_background_collision: Some(collision.clone()),
                    ..crate::trace::TraceRecord::default()
                })
                .collect(),
        }
    }

    let contact_identity = |trace| {
        crate::semantic_novelty::SemanticNoveltyDescriptor::from_trace(&trace, Vec::new())
            .unwrap()
            .axis_identities()
            .contacts
            .unwrap()
    };
    let reference = contact_identity(contact_trace(1, 7, 1));
    assert_eq!(reference, contact_identity(contact_trace(1, 99, 3)));
    assert_ne!(reference, contact_identity(contact_trace(2, 7, 1)));
}
