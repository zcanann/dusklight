use super::*;

const SOURCE: &str = r#"
milestones 1.0

milestone boot_ready {
  phase pre_input
  stable 2
  when boundary.kind == "boot" && boundary.index == 0 && player.exists && player.is_link && event.id == -1 && !next_stage.enabled
}

milestone "leave_f_sp103" {
  phase post_sim
  when (stage.name == "F_SP103" && player.procedure == "PROC_WAIT" && player.speed >= 0.0) || (next_stage.enabled && next_stage.name == "F_SP104")
}
"#;

#[test]
fn source_ast_format_binary_and_json_round_trip() {
    let program = parse(SOURCE).unwrap();
    assert_eq!(program.definitions.len(), 2);
    assert_eq!(program.definitions[0].stable_ticks, 2);
    assert_eq!(program.definitions[1].stable_ticks, 1);

    let formatted = format(&program).unwrap();
    assert_eq!(parse(&formatted).unwrap(), program);
    let json = serde_json::to_vec(&program).unwrap();
    assert_eq!(
        serde_json::from_slice::<MilestoneProgram>(&json).unwrap(),
        program
    );

    let compiled = compile(&program).unwrap();
    assert_eq!(&compiled.bytes[..4], b"DMSP");
    assert_ne!(compiled.program_sha256, [0; 32]);
    assert_eq!(compiled.definitions.len(), 2);
    assert_ne!(
        compiled.definitions[0].sha256,
        compiled.definitions[1].sha256
    );
    let decoded = decode(&compiled.bytes).unwrap();
    assert_eq!(decoded.program, program);
    assert_eq!(decoded.program_sha256, compiled.program_sha256);
    assert_eq!(decoded.definitions, compiled.definitions);
    assert_eq!(compile(&decoded.program).unwrap().bytes, compiled.bytes);
}

#[test]
fn precedence_parentheses_and_boolean_shorthand_are_exact() {
    let source = r#"milestones 1.0
milestone p {
 phase post_sim
 when player.exists || event.running && !(next_stage.enabled || boundary.reached == false)
}"#;
    let parsed = parse(source).unwrap();
    let Expression::Or(_, right) = &parsed.definitions[0].when else {
        panic!("or must have lowest precedence");
    };
    assert!(matches!(right.as_ref(), Expression::And(..)));
    let formatted = format(&parsed).unwrap();
    assert_eq!(parse(&formatted).unwrap(), parsed);
}

#[test]
fn field_types_symbols_and_properties_are_strict() {
    for invalid in [
        SOURCE.replace("event.id == -1", "event.id == 1.5"),
        SOURCE.replace("player.speed >= 0.0", "player.speed >= NaN"),
        SOURCE.replace("player.exists", "player.exists > true"),
        SOURCE.replace("stage.name == \"F_SP103\"", "stage.name == 3"),
        SOURCE.replace("F_SP103", "f/sp103"),
        SOURCE.replace("PROC_WAIT", "WAIT"),
        SOURCE.replace("boundary.kind == \"boot\"", "boundary.kind == \"startup\""),
        SOURCE.replace("player.is_link", "player.is_zelda"),
        SOURCE.replace("phase pre_input", "phase whenever"),
        SOURCE.replace("stable 2", "stable 0"),
        SOURCE.replace("stable 2", "stable 2\n  mystery no"),
        SOURCE.replace("phase pre_input", "phase pre_input\n  phase post_sim"),
    ] {
        assert!(
            parse(&invalid).is_err(),
            "unexpectedly accepted:\n{invalid}"
        );
    }
    let duplicate = format!(
        "{SOURCE}\n{}",
        &SOURCE[SOURCE.find("milestone boot_ready").unwrap()..]
    );
    assert!(parse(&duplicate).is_err());

    let alias = parse(&SOURCE.replace("PROC_WAIT", "crawl_start")).unwrap();
    assert!(format(&alias).unwrap().contains("PROC_CRAWL_START"));
    assert!(parse(&SOURCE.replace("PROC_WAIT", "crawl")).is_err());
}

#[test]
fn ast_validation_rejects_nonfinite_depth_operations_and_capacity() {
    let mut program = parse(SOURCE).unwrap();
    program.definitions[1].when = Expression::Compare {
        field: Field::PlayerSpeed,
        operator: Comparison::Equal,
        value: Value::F32(f32::NAN),
    };
    assert!(compile(&program).is_err());

    let mut deep = Expression::Compare {
        field: Field::PlayerExists,
        operator: Comparison::Equal,
        value: Value::Bool(true),
    };
    for _ in 0..MAX_EXPRESSION_DEPTH {
        deep = Expression::Not(Box::new(deep));
    }
    program.definitions[1].when = deep;
    assert!(compile(&program).is_err());

    let predicate = Expression::Compare {
        field: Field::EventId,
        operator: Comparison::Equal,
        value: Value::I32(-1),
    };
    let mut excessive = predicate.clone();
    for _ in 0..MAX_OPS {
        excessive = Expression::And(Box::new(excessive), Box::new(predicate.clone()));
    }
    program.definitions[1].when = excessive;
    assert!(compile(&program).is_err());

    let definition = parse(SOURCE).unwrap().definitions[0].clone();
    program.definitions = vec![definition; MAX_DEFINITIONS + 1];
    for (index, definition) in program.definitions.iter_mut().enumerate() {
        definition.name = format!("milestone-{index}");
    }
    assert!(compile(&program).is_err());
}

#[test]
fn every_truncation_and_authenticated_unknown_opcode_is_rejected() {
    let compiled = compile(&parse(SOURCE).unwrap()).unwrap();
    for length in 0..compiled.bytes.len() {
        assert!(
            decode(&compiled.bytes[..length]).is_err(),
            "accepted {length}"
        );
    }

    let mut corrupted = compiled.bytes.clone();
    let record_start = HEADER_BYTES;
    let name_len = u16::from_le_bytes(
        corrupted[record_start + 4..record_start + 6]
            .try_into()
            .unwrap(),
    ) as usize;
    let metadata_start = record_start + 4;
    let digest_start = metadata_start + 2 + name_len + 1 + 1 + 2 + 2 + 4;
    let bytecode_start = digest_start + 32;
    corrupted[bytecode_start] = 0xff;
    let record_len = u32::from_le_bytes(
        corrupted[record_start..record_start + 4]
            .try_into()
            .unwrap(),
    ) as usize;
    let record_end = record_start + 4 + record_len;
    let mut identity = corrupted[metadata_start..digest_start].to_vec();
    identity.extend_from_slice(&corrupted[bytecode_start..record_end]);
    let definition_digest: [u8; 32] = Sha256::new()
        .chain_update(DEFINITION_DOMAIN)
        .chain_update(identity)
        .finalize()
        .into();
    corrupted[digest_start..bytecode_start].copy_from_slice(&definition_digest);
    let digest = program_digest(&corrupted);
    corrupted[20..52].copy_from_slice(&digest);
    assert!(
        decode(&corrupted)
            .unwrap_err()
            .0
            .contains("unknown milestone opcode")
    );
}

#[test]
fn identity_covers_name_phase_stability_and_expression() {
    let base = parse(SOURCE).unwrap();
    let identity = compile(&base).unwrap();
    for mutate in [
        |program: &mut MilestoneProgram| program.definitions[0].name.push('x'),
        |program: &mut MilestoneProgram| program.definitions[0].phase = EvaluationPhase::PostSim,
        |program: &mut MilestoneProgram| program.definitions[0].stable_ticks += 1,
        |program: &mut MilestoneProgram| {
            program.definitions[0].when = Expression::Compare {
                field: Field::PlayerExists,
                operator: Comparison::Equal,
                value: Value::Bool(false),
            }
        },
    ] {
        let mut changed = base.clone();
        mutate(&mut changed);
        let changed = compile(&changed).unwrap();
        assert_ne!(changed.program_sha256, identity.program_sha256);
        assert_ne!(
            changed.definitions[0].sha256,
            identity.definitions[0].sha256
        );
    }
}

#[test]
fn wire_field_ids_and_opcodes_are_stable() {
    assert_eq!(Field::BoundaryKind as u8, 1);
    assert_eq!(Field::EventId as u8, 15);
    assert_eq!(Field::PlayerIsLink as u8, 21);
    assert_eq!(Field::NextStageEnabled as u8, 22);
    let program =
        parse("milestones 1.0 milestone one { phase pre_input when event.id == -1 }").unwrap();
    let bytes = compile(&program).unwrap().bytes;
    let bytecode_start = HEADER_BYTES + 4 + RECORD_FIXED_BYTES + "one".len();
    assert_eq!(
        &bytes[bytecode_start..bytecode_start + 8],
        &[0x01, 15, 0x13, 0xff, 0xff, 0xff, 0xff, 0x20]
    );
}

#[test]
fn language_1_1_types_flags_timers_hashes_rng_and_collision_facts() {
    let source = r#"milestones 1.1
milestone rich {
  phase post_sim
  stable 3
  when player.actor_name == 253 && player.velocity.y <= 0.0 &&
   player.mode_flags has_all 1024 && player.mode_flags has_any 1028 &&
   player.timer.damage_wait == 0 && player.timer.sword_change_wait <= 4 &&
   event.mode == 2 && event.status >= 1 && event.name_hash.present &&
   event.name_hash.fnv1a32 == 305419896 && rng.primary.state0 == 11 &&
   rng.secondary.calls >= 200 && collision.ground.contact &&
   collision.ground.clearance <= 0.5
}"#;
    let program = parse(source).unwrap();
    let compiled = compile(&program).unwrap();
    assert_eq!(&compiled.bytes[4..12], &[1, 0, 1, 0, 1, 0, 1, 0]);
    let decoded = decode(&compiled.bytes).unwrap();
    assert_eq!(decoded.program, program);
    let formatted = format(&program).unwrap();
    assert!(formatted.contains("player.mode_flags has_all 1024"));
    assert!(formatted.contains("player.mode_flags has_any 1028"));
    assert_eq!(parse(&formatted).unwrap(), program);

    assert!(parse(&source.replace("milestones 1.1", "milestones 1.0")).is_err());
    assert!(parse(&source.replace("has_all 1024", "has_all 0")).is_err());
    assert!(
        parse(&source.replace(
            "collision.ground.contact",
            "collision.ground.contact has_any true"
        ))
        .is_err()
    );
    assert_eq!(Field::PlayerModeFlags as u8, 34);
    assert_eq!(Field::EventNameHash as u8, 42);
    assert_eq!(Field::CollisionGroundClearance as u8, 58);
    assert_eq!(Comparison::HasAll as u8, 0x26);
    assert_eq!(Comparison::HasAny as u8, 0x27);
}

#[test]
fn language_1_2_stable_actor_queries_geometry_and_indexed_flags_round_trip() {
    let source = r#"milestones 1.2

milestone local_actor_goal {
  phase post_sim
  stable 4
  when actor.placed.exists("F_SP103", -1, 7, 42) &&
   actor.placed.position.y("F_SP103", -1, 7, 42) >= -20.0 &&
   actor.placed.distance_to_player("F_SP103", -1, 7, 42) <= 125.5 &&
   actor.placed.current_room("F_SP103", -1, 7, 42) == 0 &&
   actor.placed.health("F_SP103", -1, 7, 42) > 0 &&
   actor.placed.status("F_SP103", -1, 7, 42) has_any 4 &&
   flag.event(821) && flag.temporary(184) == false &&
   flag.dungeon(63) && flag.switch(0, 239)
}
"#;
    let program = parse(source).unwrap();
    let formatted = format(&program).unwrap();
    assert_eq!(parse(&formatted).unwrap(), program);
    assert!(formatted.contains("actor.placed.distance_to_player(\"F_SP103\", -1, 7, 42)"));
    assert!(formatted.contains("flag.switch(0, 239) == true"));

    let compiled = compile(&program).unwrap();
    assert_eq!(&compiled.bytes[4..12], &[1, 0, 2, 0, 1, 0, 2, 0]);
    assert!(compiled.bytes.contains(&0x02));
    let decoded = decode(&compiled.bytes).unwrap();
    assert_eq!(decoded.program, program);
    assert_eq!(compile(&decoded.program).unwrap().bytes, compiled.bytes);

    for invalid in [
        source.replace("milestones 1.2", "milestones 1.1"),
        source.replace("F_SP103\", -1", "bad-stage\", -1"),
        source.replace("flag.event(821)", "flag.event(822)"),
        source.replace("flag.temporary(184)", "flag.temporary(185)"),
        source.replace("flag.dungeon(63)", "flag.dungeon(64)"),
        source.replace("flag.switch(0, 239)", "flag.switch(0, 240)"),
        source.replace(", 7, 42)", ", 65535, 42)"),
    ] {
        assert!(
            parse(&invalid).is_err(),
            "accepted invalid source: {invalid}"
        );
    }
}

#[test]
fn language_1_3_ranges_regions_planes_transitions_and_sequences_round_trip() {
    let source = r#"milestones 1.3

milestone crossed_plane_after_contact {
  phase post_sim
  within 4
  when collision.ground.contact && player.position.x between -5.0 and 5.0
  then player.in_aabb(-10.0, -20.0, -30.0, 10.0, 20.0, 30.0)
  then event.id == 17
  then player.plane_signed_distance(0.0, 0.0, 0.0, 1.0, 0.0, 0.0) >= 0.0
}

milestone exact_next_tick_transition {
  phase post_sim
  within 1
  when player.procedure == 7
  then player.procedure == 8
}
"#;
    let program = parse(source).unwrap();
    assert_eq!(program.definitions[0].then.len(), 3);
    assert_eq!(program.definitions[0].within_ticks, Some(4));
    let formatted = format(&program).unwrap();
    assert!(!formatted.contains(" between "));
    assert!(formatted.contains("player.position.x >= -5.0 && player.position.x <= 5.0"));
    assert!(formatted.contains("player.in_aabb(-10.0, -20.0, -30.0, 10.0, 20.0, 30.0)"));
    assert_eq!(parse(&formatted).unwrap(), program);

    let compiled = compile(&program).unwrap();
    assert_eq!(&compiled.bytes[4..12], &[1, 0, 3, 0, 1, 0, 3, 0]);
    assert!(compiled.bytes.contains(&0x40));
    assert!(compiled.bytes.contains(&0x41));
    let decoded = decode(&compiled.bytes).unwrap();
    assert_eq!(decoded.program, program);
    assert_eq!(compile(&decoded.program).unwrap().bytes, compiled.bytes);

    for invalid in [
        source.replace("milestones 1.3", "milestones 1.2"),
        source.replace("  within 4\n", ""),
        source.replace("  then player.procedure == 8\n", ""),
        source.replace("  within 4", "  stable 2\n  within 4"),
        source.replace(
            "player.in_aabb(-10.0, -20.0, -30.0, 10.0, 20.0, 30.0)",
            "player.in_aabb(10.0, -20.0, -30.0, -10.0, 20.0, 30.0)",
        ),
        source.replace(
            "player.plane_signed_distance(0.0, 0.0, 0.0, 1.0, 0.0, 0.0)",
            "player.plane_signed_distance(0.0, 0.0, 0.0, 0.0, 0.0, 0.0)",
        ),
        source.replace("between -5.0 and 5.0", "between 5.0 and -5.0"),
    ] {
        assert!(
            parse(&invalid).is_err(),
            "accepted invalid source: {invalid}"
        );
    }
}

#[test]
fn language_1_4_named_value_projections_round_trip() {
    let source =
        include_str!("../../../../../../tests/fixtures/automation/value_projection.milestones");
    let program = parse(source).unwrap();
    let projections = &program.definitions[0].projections;
    assert_eq!(projections.len(), 1);
    assert_eq!(projections[0].name, "handoff-state");
    assert_eq!(projections[0].items.len(), 5);
    let projection_identity = value_projection_identity(&projections[0]).unwrap();
    let projection_identity_hex = projection_identity
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    assert_eq!(
        projection_identity_hex,
        "acb5c9cd5570ebe610e321a3f5a33856a6af7cfaaf808b5b394f471170fcf5f3"
    );
    let formatted = format(&program).unwrap();
    assert_eq!(parse(&formatted).unwrap(), program);

    let compiled = compile(&program).unwrap();
    assert_eq!(&compiled.bytes[4..12], &[1, 0, 4, 0, 1, 0, 4, 0]);
    for opcode in 0x50..=0x53 {
        assert!(compiled.bytes.contains(&opcode));
    }
    let decoded = decode(&compiled.bytes).unwrap();
    assert_eq!(decoded.program, program);
    assert_eq!(compile(&decoded.program).unwrap().bytes, compiled.bytes);

    for invalid in [
        source.replace("milestones 1.4", "milestones 1.3"),
        source.replace("    rng secondary\n", "    rng primary\n"),
        source.replace("flag event 821", "flag event 822"),
        source.replace(
            "actor_population \"F_SP103\" 1",
            "actor_population \"bad\" 1",
        ),
    ] {
        assert!(
            parse(&invalid).is_err(),
            "accepted invalid source: {invalid}"
        );
    }
}

#[test]
fn language_1_6_interaction_identity_facts_round_trip_and_evaluate_offline() {
    let source = r#"milestones 1.6

milestone exact_interaction {
  phase post_sim
  when player.interaction.do_status == 21 &&
   player.interaction.talk_partner.exists &&
   player.interaction.talk_partner.actor_name == 42 &&
   player.interaction.talk_partner.set_id == 7 &&
   player.interaction.talk_partner.home_room == 1 &&
   player.interaction.talk_partner.current_room == 2 &&
   player.interaction.talk_partner.home_position.x between 9.5 and 10.5 &&
   player.interaction.talk_partner.home_position.y == 20.0 &&
   player.interaction.talk_partner.home_position.z == 30.0 &&
   player.interaction.grabbed_actor.exists &&
   player.interaction.grabbed_actor.actor_name == 43 &&
   player.interaction.grabbed_actor.set_id == 8 &&
   player.interaction.grabbed_actor.home_room == 3 &&
   player.interaction.grabbed_actor.current_room == 4 &&
   player.interaction.grabbed_actor.home_position.x == 40.0 &&
   player.interaction.grabbed_actor.home_position.y == 50.0 &&
   player.interaction.grabbed_actor.home_position.z == 60.0
}
"#;
    let program = parse(source).unwrap();
    assert_eq!(parse(&format(&program).unwrap()).unwrap(), program);
    let compiled = compile(&program).unwrap();
    assert_eq!(&compiled.bytes[4..12], &[1, 0, 6, 0, 1, 0, 6, 0]);
    assert_eq!(decode(&compiled.bytes).unwrap().program, program);
    assert!(parse(&source.replace("milestones 1.6", "milestones 1.5")).is_err());

    let mut record = crate::trace::TraceRecord {
        boundary_index: 1,
        simulation_tick: 0,
        observation_phase: crate::trace::TracePhase::PostSimulation,
        ..crate::trace::TraceRecord::default()
    };
    record.channel_status.insert(
        crate::trace::TraceChannel::PlayerAction,
        crate::trace::TraceChannelStatus::Present,
    );
    record.player_action = Some(crate::trace::TracePlayerAction {
        procedure_id: 0,
        mode_flags: 0,
        procedure_context_raw: [0; 6],
        damage_wait_timer: 0,
        sword_at_up_time: 0,
        ice_damage_wait_timer: 0,
        sword_change_wait_timer: 0,
        under_animations: std::array::from_fn(|_| crate::trace::TraceAnimationLane {
            resource_id: 0,
            frame: 0.0,
            rate: 0.0,
        }),
        upper_animations: std::array::from_fn(|_| crate::trace::TraceAnimationLane {
            resource_id: 0,
            frame: 0.0,
            rate: 0.0,
        }),
        do_status: 21,
        talk_partner: Some(crate::trace::TraceActorIdentity {
            session_process_id: 100,
            actor_name: 42,
            set_id: 7,
            home_room: 1,
            current_room: 2,
            home_position: Some([10.0, 20.0, 30.0]),
        }),
        grabbed_actor: Some(crate::trace::TraceActorIdentity {
            session_process_id: 101,
            actor_name: 43,
            set_id: 8,
            home_room: 3,
            current_room: 4,
            home_position: Some([40.0, 50.0, 60.0]),
        }),
    });
    let trace = crate::trace::DecodedTrace {
        version: 5,
        boot: crate::tape::TapeBoot::Process,
        tick_rate_numerator: 30,
        tick_rate_denominator: 1,
        requested_channels: crate::trace::TraceChannel::PlayerAction.bit(),
        capacity_exhausted: false,
        retention: None,
        channel_formats: BTreeMap::new(),
        records: vec![record],
    };
    assert!(evaluate_recorded_trace(&program, &trace).unwrap()["exact_interaction"].is_some());

    let mut wrong = trace;
    wrong.records[0]
        .player_action
        .as_mut()
        .unwrap()
        .talk_partner
        .as_mut()
        .unwrap()
        .set_id = 9;
    assert!(evaluate_recorded_trace(&program, &wrong).unwrap()["exact_interaction"].is_none());

    let absence_program = parse(
        r#"milestones 1.5

milestone no_talk_partner {
  phase post_sim
  when !player.interaction.talk_partner.exists
}
"#,
    )
    .unwrap();
    let mut absent_action = wrong.records[0].player_action.clone().unwrap();
    absent_action.talk_partner = None;
    wrong.records[0].player_action = None;
    wrong.records[0].channel_status.insert(
        crate::trace::TraceChannel::PlayerAction,
        crate::trace::TraceChannelStatus::Truncated,
    );
    assert!(
        evaluate_recorded_trace(&absence_program, &wrong).unwrap()["no_talk_partner"].is_none()
    );
    wrong.records[0].channel_status.insert(
        crate::trace::TraceChannel::PlayerAction,
        crate::trace::TraceChannelStatus::Present,
    );
    wrong.records[0].player_action = Some(absent_action);
    assert!(
        evaluate_recorded_trace(&absence_program, &wrong).unwrap()["no_talk_partner"].is_some()
    );
}

#[test]
fn required_facts_are_derived_only_from_objective_truth_expressions() {
    let program = parse(
        r#"milestones 1.3

milestone target {
  phase post_sim
  when stage.name == "F_SP104" && player.exists &&
   player.in_aabb(-1.0, -2.0, -3.0, 1.0, 2.0, 3.0)
  then event.running
  within 5
}
"#,
    )
    .unwrap();
    assert_eq!(
        required_query_facts(&program, "target").unwrap(),
        [
            "event.running",
            "player.exists",
            "player.in_aabb",
            "stage.name",
        ]
    );
    assert!(required_query_facts(&program, "missing").is_err());
}
