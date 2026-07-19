use super::*;
use crate::option_diagnostics::{
    ClampDisposition, IntendedTarget, OptionContact, OptionDiagnostic, OptionDiagnosticBundle,
    OptionGoalProgress, OptionTickDiagnostic, ViewportPoint,
};
use crate::option_execution::{
    OptionCondition, OptionEndReason, OptionExecution, OptionType, TapeRange,
};
use crate::tape::{InputFrame, RawPadState};

#[test]
fn explicit_stage_boot_supersedes_the_legacy_workbench_stage_argument() {
    let mut tape = InputTape::default();
    assert_eq!(
        legacy_seed_stage(&tape, crate::search::SegmentProfile::Fsp103ToFsp104),
        Some("F_SP103,1,1,3")
    );
    tape.boot = TapeBoot::Stage {
        stage: "F_SP103".into(),
        room: 1,
        point: 1,
        layer: 3,
        save_slot: None,
        fixture: None,
    };
    assert_eq!(
        legacy_seed_stage(&tape, crate::search::SegmentProfile::Fsp103ToFsp104),
        None
    );
}

fn temporary_root(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("huntctl-workbench-{name}-{nonce}"));
    fs::create_dir_all(&root).unwrap();
    root
}

fn write_tape(root: &Path, name: &str, values: &[i8]) {
    let tape = InputTape {
        frames: values
            .iter()
            .map(|value| InputFrame {
                owned_ports: 0x0f,
                pads: [
                    RawPadState {
                        stick_x: *value,
                        ..RawPadState::default()
                    },
                    RawPadState::default(),
                    RawPadState::default(),
                    RawPadState::default(),
                ],
                ..InputFrame::default()
            })
            .collect(),
        ..InputTape::default()
    };
    fs::write(root.join(name), tape.encode().unwrap()).unwrap();
}

fn timeline() -> Timeline {
    Timeline::parse(
            r#"
timeline test
segment boot_link.one root profile boot_to_fsp103 uses tape first.tape starts clean produces aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
label boot_link.one "Boot to Link"
segment link_exit.one after boot_link.one profile fsp103_to_fsp104 uses tape second.tape starts aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa produces bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
continuation main starts root@clean
continue main with boot_link.one after root@clean
continue main with link_exit.one after boot_link.one@aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
"#,
        )
        .unwrap()
}

#[test]
fn completed_search_elites_project_as_ephemeral_structural_siblings() {
    let root = temporary_root("generated-search");
    let search_root = root.join("build/search");
    let run = search_root.join("route-run");
    let generation = run.join("g000");
    let candidate_id = "c".repeat(64);
    let attempt_root = generation
        .join("evaluations/candidates")
        .join(&candidate_id)
        .join("attempt-001");
    fs::create_dir_all(&attempt_root).unwrap();
    fs::create_dir_all(
        generation
            .join("evaluations/candidates")
            .join(&candidate_id)
            .join("attempt-002"),
    )
    .unwrap();
    let suffix = generation.join(format!("{candidate_id}.tape"));
    let candidate = generation.join(format!("{candidate_id}.candidate.json"));
    let full = attempt_root.join("full.tape");
    let tape = InputTape {
        frames: vec![InputFrame::default(); 9],
        ..InputTape::default()
    };
    fs::write(&suffix, tape.encode().unwrap()).unwrap();
    fs::write(&full, tape.encode().unwrap()).unwrap();
    fs::write(&candidate, b"{}").unwrap();
    let objective = serde_json::json!({
        "schema":"dusklight-anchored-search-objective/v2",
        "segment":"fsp103_to_fsp104",
        "digest":"1".repeat(64),
        "prefix_sha256":"2".repeat(64),
        "prefix_frames":3,
        "milestone_program_sha256":"3".repeat(64),
        "game_sha256":"4".repeat(64),
        "dvd_sha256":"5".repeat(64),
        "source_milestone":"control",
        "source_definition_sha256":"6".repeat(64),
        "source_boundary_fingerprint":"a".repeat(32),
        "source_tape_frame":2,
        "source_boundary_index":3,
        "goal_milestone":"exit",
        "goal_definition_sha256":"7".repeat(64)
    });
    fs::write(
        generation.join("results.json"),
        serde_json::to_vec(&serde_json::json!({
            "schema":"dusklight-anchored-search-results/v2",
            "objective":objective.clone(),
            "results":{
                "schema":"dusklight-search-results/v1",
                "segment":"fsp103_to_fsp104",
                "candidates":{
                    (candidate_id.clone()):{
                        "milestone_depth":2,
                        "attempts":2,
                        "successes":2,
                        "first_hit_ticks":[7,7]
                    }
                }
            }
        }))
        .unwrap(),
    )
    .unwrap();
    for attempt in 1..=2 {
        let attempt_dir = generation
            .join("evaluations/candidates")
            .join(&candidate_id)
            .join(format!("attempt-{attempt:03}"));
        let attempt_tape = if attempt == 1 {
            full.clone()
        } else {
            let path = attempt_dir.join("full.tape");
            fs::write(&path, tape.encode().unwrap()).unwrap();
            path
        };
        fs::write(
            attempt_dir.join("attempt.json"),
            serde_json::to_vec(&serde_json::json!({
                "candidate_id":candidate_id,
                "tape":attempt_tape,
                "exit_code":0,
                "infrastructure_error":null,
                "first_hit_tick":7,
                "goal_reached":true,
                "boundary_fingerprints":{
                    "exit":{
                        "schema":"dusklight.milestone-boundary/v1",
                        "algorithm":"xxh3-128",
                        "canonical_encoding":"little-endian-fixed-v1",
                        "digest":"d".repeat(32)
                    }
                }
            }))
            .unwrap(),
        )
        .unwrap();
    }
    let route_source = format!(
        r#"
timeline generated
predicate_program milestones.milestones
segment parent root profile boot_to_fsp103 uses tape parent.tape starts clean produces {}
segment reference after parent profile fsp103_to_fsp104 uses tape reference.tape starts {} produces {}
label reference "To exit"
goal exit_goal on reference predicate exit
continuation main starts root@clean
continue main with parent after root@clean
continue main with reference after parent@{}
"#,
        "a".repeat(32),
        "a".repeat(32),
        "b".repeat(32),
        "a".repeat(32)
    );
    let route = Timeline::parse(&route_source).unwrap();
    let timeline_path = root.join("route.timeline");
    fs::write(&timeline_path, &route_source).unwrap();
    let projected = generated_search_projections(&route, &search_root);
    assert_eq!(projected.len(), 1);
    assert_eq!(projected[0].segment.parent.as_deref(), Some("parent"));
    assert_eq!(projected[0].segment.first_hit_tick, Some(7));
    assert_eq!(
        projected[0].segment.name.as_deref(),
        Some("To exit · 7f · cccccc")
    );
    assert_eq!(
        projected[0]
            .segment
            .generated
            .as_ref()
            .unwrap()
            .proof_attempts,
        2
    );
    let state = root.join("state");
    let preview = preview_sibling_deletion(&timeline_path, &root, &state, "reference").unwrap();
    assert!(preview.segments.is_empty());
    assert!(preview.draft_roots.is_empty());
    assert_eq!(preview.generated.len(), 1);
    assert_eq!(preview.generated[0].candidate_id, candidate_id);
    let result = apply_sibling_deletion(
        &timeline_path,
        &root,
        &state,
        &BrowserSiblingDeleteApplyRequest {
            keep_id: "reference".into(),
            confirmation_token: preview.confirmation_token,
        },
    )
    .unwrap();
    assert!(result.segments.is_empty());
    assert_eq!(result.generated_candidates, vec![candidate_id.clone()]);
    assert!(
        candidate.is_file(),
        "search artifacts must remain recoverable"
    );
    assert!(
        visible_generated_search_projections(&route, &search_root, &state)
            .unwrap()
            .is_empty()
    );
    assert!(
        load_generated_search_tombstones(&state)
            .unwrap()
            .candidate_ids
            .contains(&candidate_id)
    );
    fs::remove_file(generation.join("results.json")).unwrap();
    assert!(generated_search_projections(&route, &search_root).is_empty());
    fs::remove_dir_all(root).unwrap();
}

const MILESTONE_SOURCE: &str = r#"milestones 1.0

milestone boot {
  phase pre_input
  when boundary.kind == "boot"
}

milestone control {
  phase post_sim
  when stage.name == "F_SP103" && player.exists
}

milestone exit {
  phase post_sim
  stable 2
  when stage.name == "F_SP104"
}
"#;

fn hex_digest(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn milestone_timeline_source() -> String {
    let compiled = milestone_dsl::compile_source(MILESTONE_SOURCE).unwrap();
    let program = hex_digest(&compiled.program_sha256);
    let control = hex_digest(
        &compiled
            .definitions
            .iter()
            .find(|definition| definition.name == "control")
            .unwrap()
            .sha256,
    );
    let exit = hex_digest(
        &compiled
            .definitions
            .iter()
            .find(|definition| definition.name == "exit")
            .unwrap()
            .sha256,
    );
    format!(
        r#"
timeline test
predicate_program route.milestones
origin boot predicate boot
segment boot_link.one root profile boot_to_fsp103 uses tape first.tape starts clean produces aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
segment link_exit.one after boot_link.one profile fsp103_to_fsp104 uses tape second.tape starts aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa produces bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
goal control on boot_link.one predicate control
goal exit on link_exit.one predicate exit
proof boot_link.one satisfies control program {program} predicate {control} ticks 2
proof link_exit.one satisfies exit program {program} predicate {exit} ticks 1
continuation main starts root@clean
continue main with boot_link.one after root@clean
continue main with link_exit.one after boot_link.one@aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
"#
    )
}

fn sibling_timeline_source() -> String {
    r#"
timeline siblings
segment root root profile boot_to_fsp103 uses tape root.tape starts clean produces aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
segment left after root profile fsp103_to_fsp104 uses tape left.tape starts aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa produces bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
segment left_child after left profile fsp103_to_fsp104 uses tape left-child.tape starts bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb produces cccccccccccccccccccccccccccccccc
segment keep after root profile fsp103_to_fsp104 uses tape keep.tape starts aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa produces dddddddddddddddddddddddddddddddd
segment keep_child after keep profile fsp103_to_fsp104 uses tape keep-child.tape starts dddddddddddddddddddddddddddddddd produces eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee
segment right after root profile fsp103_to_fsp104 uses tape right.tape starts aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa produces ffffffffffffffffffffffffffffffff
"#
        .into()
}

fn sibling_timeline_with_shared_goal_source() -> String {
    let digest = "11".repeat(32);
    format!(
        r#"
timeline siblings
predicate_program sibling.milestones
segment root root profile boot_to_fsp103 uses tape root.tape starts clean produces aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
segment incumbent after root profile fsp103_to_fsp104 uses tape incumbent.tape starts aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa produces bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
segment keep after root profile fsp103_to_fsp104 uses tape keep.tape starts aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa produces cccccccccccccccccccccccccccccccc
segment unrelated_profile after root profile link_control_to_tunnel_crawl_start uses tape unrelated.tape starts aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa produces dddddddddddddddddddddddddddddddd
goal destination on incumbent predicate destination
proof incumbent satisfies destination program {digest} predicate {digest} ticks 150
proof keep satisfies destination program {digest} predicate {digest} ticks 129
continuation main starts root@clean
continue main with root after root@clean
continue main with incumbent after root@aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
"#
    )
}

fn timeline_with_milestone_program(root: &Path) -> Timeline {
    fs::write(root.join("route.milestones"), MILESTONE_SOURCE).unwrap();
    Timeline::parse(&milestone_timeline_source()).unwrap()
}

fn call_http(config: &WorkbenchConfig, method: &str, path: &str, body: &[u8]) -> HttpResponse {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {address}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        body.len()
    );
    let body = body.to_vec();
    let client = thread::spawn(move || {
        let mut stream = TcpStream::connect(address).unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        stream.write_all(&body).unwrap();
        stream.shutdown(std::net::Shutdown::Write).unwrap();
    });
    let (mut stream, _) = listener.accept().unwrap();
    let response = handle_http(&mut stream, address, config);
    client.join().unwrap();
    response
}

#[test]
fn launches_use_the_compiled_authored_program_and_native_result_stream() {
    let root = temporary_root("milestone-launch");
    let route = timeline_with_milestone_program(&root);
    let state = root.join("state");
    fs::create_dir(&state).unwrap();
    let mut command = Command::new("game");
    append_authored_milestone_args(
        &route,
        &root,
        &state,
        &mut command,
        Some("gameplay-ready-f-sp103"),
    )
    .unwrap();

    let arguments = command
        .get_args()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert_eq!(arguments[0], "--milestone-program");
    assert_eq!(arguments[2], "--milestones");
    assert_eq!(arguments[3], "boot,control,exit,gameplay-ready-f-sp103");
    assert_eq!(arguments[4], "--milestone-result");
    let decoded = milestone_dsl::decode(&fs::read(&arguments[1]).unwrap()).unwrap();
    assert_eq!(decoded.definitions.len(), 3);
    assert_eq!(
        arguments[5],
        state.join("route-milestones.json").display().to_string()
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn graph_exposes_timeline_shape_and_scrub_ranges() {
    let root = temporary_root("graph");
    write_tape(&root, "first.tape", &[1, 2, 3, 4]);
    write_tape(&root, "second.tape", &[5, 6, 7]);
    let graph = graph_from_timeline(&timeline(), &root).unwrap();
    assert_eq!(graph.schema, "dusklight.route-workbench.graph.v9");
    assert!(graph.origin.is_none());
    assert_eq!(graph.segments.len(), 2);
    assert!(graph.segments.iter().all(|segment| segment.playable));
    assert!(graph.segments.iter().all(|segment| !segment.recordable));
    assert_eq!(graph.segments[0].parent, None);
    assert_eq!(graph.segments[0].name.as_deref(), Some("Boot to Link"));
    assert_eq!(graph.segments[1].parent.as_deref(), Some("boot_link.one"));
    let playback = materialize_segment_chain(&timeline(), &root, "link_exit.one").unwrap();
    assert_eq!(playback.tape.frames.len(), 7);
    assert_eq!(playback.steps[1].chain_start_frame, 4);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn graph_projects_authenticated_option_sidecars() {
    let root = temporary_root("graph-option-sidecar");
    write_tape(&root, "first.tape", &[1, 2, 3, 4]);
    write_tape(&root, "second.tape", &[5, 6, 7]);
    let bytes = fs::read(root.join("first.tape")).unwrap();
    let tape = InputTape::decode(&bytes).unwrap().tape;
    let execution = OptionExecution::capture(
        "walk-to-door".into(),
        OptionType::Move,
        BTreeMap::new(),
        1,
        4,
        OptionCondition::TargetReached {
            target: "door".into(),
        },
        Vec::new(),
        OptionEndReason::Completed,
        &tape,
        TapeRange {
            start_frame: 0,
            end_frame_exclusive: 4,
        },
    )
    .unwrap();
    let ticks = execution
        .emitted_raw_actions
        .iter()
        .cloned()
        .enumerate()
        .map(|(index, raw_output)| OptionTickDiagnostic {
            local_tick: index as u32,
            error_vector_f32_bits: None,
            action_mask: None,
            raw_output: raw_output.clone(),
            clamps: ClampDisposition::default(),
            game_consumed_input: raw_output,
            contacts: vec![OptionContact {
                kind: "floor".into(),
                position_f32_bits: [0, 0, 0],
                normal_f32_bits: Some([0, 1.0_f32.to_bits(), 0]),
                stable_surface_id: Some("room0-floor".into()),
                viewport: Some(ViewportPoint {
                    x_u16: 12_000,
                    y_u16: 48_000,
                }),
            }],
            goal_progress: vec![OptionGoalProgress {
                goal: "door".into(),
                value_f32_bits: Some((index as f32 / 3.0).to_bits()),
                satisfied: index == 3,
            }],
            target_viewport: Some(ViewportPoint {
                x_u16: 50_000,
                y_u16: 22_000,
            }),
        })
        .collect();
    let diagnostic = OptionDiagnostic::capture(
        execution,
        IntendedTarget::Actor {
            selector: "placed:door".into(),
            runtime_process_id: Some(42),
        },
        ticks,
    )
    .unwrap();
    let bundle = OptionDiagnosticBundle::capture(&tape, vec![diagnostic]).unwrap();
    fs::write(
        root.join("first.tape.options.json"),
        serde_json::to_vec_pretty(&bundle).unwrap(),
    )
    .unwrap();

    let graph = graph_from_timeline(&timeline(), &root).unwrap();
    let visualization = &graph.segments[0].option_visualization;
    assert_eq!(visualization.len(), 1);
    assert_eq!(visualization[0].option_id, "walk-to-door");
    assert_eq!(visualization[0].curve.len(), 4);
    assert_eq!(visualization[0].contacts.len(), 4);
    assert!(visualization[0].goal_progress[3].progress.satisfied);
    assert!(graph.segments[0].option_diagnostic_error.is_none());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn graph_exposes_predicate_source_summaries_and_proof_identity() {
    let root = temporary_root("milestone-graph");
    write_tape(&root, "first.tape", &[1, 2, 3, 4]);
    write_tape(&root, "second.tape", &[5, 6, 7]);
    let route = timeline_with_milestone_program(&root);
    let graph = graph_from_timeline(&route, &root).unwrap();
    let program = graph.predicate_program.as_ref().unwrap();
    assert_eq!(program.source, MILESTONE_SOURCE);
    assert_eq!(
        program.revision_sha256,
        source_revision(MILESTONE_SOURCE.as_bytes())
    );
    assert_eq!(program.definitions.len(), 3);
    assert_eq!(program.definitions[1].name, "control");
    assert_eq!(program.definitions[1].stable_ticks, 1);
    assert!(
        serde_json::to_value(&program.definitions[1].expression)
            .unwrap()
            .is_object()
    );
    assert!(graph.segments.iter().all(|segment| segment.playable));
    assert!(
        graph
            .segments
            .iter()
            .all(|segment| segment.predicate_proof == "verified")
    );
    assert!(graph.segments.iter().all(|segment| segment.recordable));

    let changed = MILESTONE_SOURCE.replace("F_SP104", "F_SP105");
    fs::write(root.join("route.milestones"), changed).unwrap();
    let stale = graph_from_timeline(&route, &root).unwrap();
    assert!(stale.segments.iter().all(|segment| segment.playable));
    assert!(
        stale
            .segments
            .iter()
            .all(|segment| segment.predicate_proof == "stale")
    );
    assert!(stale.segments.iter().all(|segment| !segment.recordable));
    assert!(
        stale
            .segments
            .iter()
            .all(|segment| segment.record_anchors.is_empty())
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn segment_playback_requires_a_canonical_loadable_parent_chain() {
    let root = temporary_root("strict-segment-chain");
    write_tape(&root, "child.tape", &[5, 6, 7]);
    let seeded_prefix = Timeline::parse(
            r#"
timeline seeded
segment boot_link.seed root profile boot_to_fsp103 uses baseline boot_to_fsp103 starts clean produces aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
segment link_exit.child after boot_link.seed profile fsp103_to_fsp104 uses tape child.tape starts aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa produces bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
continuation main starts root@clean
continue main with boot_link.seed after root@clean
continue main with link_exit.child after boot_link.seed@aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
"#,
        )
        .unwrap();
    let graph = graph_from_timeline(&seeded_prefix, &root).unwrap();
    let child = graph
        .segments
        .iter()
        .find(|segment| segment.id == "link_exit.child")
        .unwrap();
    assert!(!child.playable);

    let missing_prefix = Timeline::parse(
            r#"
timeline missing
segment boot_link.missing root profile boot_to_fsp103 uses tape missing.tape starts cccccccccccccccccccccccccccccccc produces aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
segment link_exit.child after boot_link.missing profile fsp103_to_fsp104 uses tape child.tape starts aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa produces bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
continuation main starts root@cccccccccccccccccccccccccccccccc
continue main with boot_link.missing after root@cccccccccccccccccccccccccccccccc
continue main with link_exit.child after boot_link.missing@aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
"#,
        )
        .unwrap();
    let graph = graph_from_timeline(&missing_prefix, &root).unwrap();
    let child = graph
        .segments
        .iter()
        .find(|segment| segment.id == "link_exit.child")
        .unwrap();
    assert!(!child.playable);

    let independent_root = Timeline::parse(
            r#"
timeline independent
segment tunnel.child root profile link_control_to_tunnel_crawl_start uses tape child.tape starts aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa produces bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
continuation main starts root@aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
continue main with tunnel.child after root@aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
"#,
        )
        .unwrap();
    let graph = graph_from_timeline(&independent_root, &root).unwrap();
    let child = &graph.segments[0];
    assert!(child.playable);
    assert!(materialize_segment_playback(&independent_root, &root, "tunnel.child", None).is_ok());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn forged_record_request_cannot_bypass_stale_predicate_proof() {
    let root = temporary_root("milestone-record-proof");
    write_tape(&root, "first.tape", &[1, 2, 3, 4]);
    write_tape(&root, "second.tape", &[5, 6, 7]);
    let route = timeline_with_milestone_program(&root);
    fs::write(root.join("route.timeline"), milestone_timeline_source()).unwrap();
    fs::write(
        root.join("route.milestones"),
        MILESTONE_SOURCE.replace("F_SP103", "F_SP105"),
    )
    .unwrap();
    fs::write(root.join("game"), b"game").unwrap();
    fs::write(root.join("disc"), b"disc").unwrap();
    let config = WorkbenchConfig {
        timeline_path: root.join("route.timeline"),
        repository_root: root.clone(),
        working_directory: root.clone(),
        game: root.join("game"),
        dvd: root.join("disc"),
        state_root: root.join("state"),
    };
    let error = record_continuation(
        &route,
        &config,
        BrowserRecordRequest {
            parent: BrowserRecordParent::Segment {
                id: "boot_link.one".into(),
                terminal_goal: "control".into(),
            },
            label: String::new(),
            countdown_seconds: DEFAULT_RECORD_INPUT_COUNTDOWN_SECONDS,
            speed_percent: 100,
        },
    )
    .unwrap_err();
    assert!(error.to_string().contains("verified goal"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn milestone_program_update_validates_parser_topology_and_stale_revision() {
    let root = temporary_root("milestone-update");
    let route = timeline_with_milestone_program(&root);
    let initial = milestone_program_projection(&route, &root)
        .unwrap()
        .unwrap();
    let replacement = MILESTONE_SOURCE.replace("stable 2", "stable 3");
    let updated = update_milestone_program(
        &route,
        &root,
        &BrowserMilestoneProgramUpdateRequest {
            owner: String::new(),
            expected_revision_sha256: initial.revision_sha256.clone(),
            source: replacement.clone(),
        },
    )
    .unwrap();
    assert_ne!(updated.revision_sha256, initial.revision_sha256);
    assert_eq!(
        fs::read_to_string(root.join("route.milestones")).unwrap(),
        replacement
    );
    assert_eq!(updated.definitions[2].stable_ticks, 3);
    assert!(fs::read_dir(&root).unwrap().all(|entry| {
        let name = entry.unwrap().file_name().to_string_lossy().into_owned();
        !name.ends_with(".tmp") && !name.ends_with(".rollback")
    }));

    let stale = update_milestone_program(
        &route,
        &root,
        &BrowserMilestoneProgramUpdateRequest {
            owner: String::new(),
            expected_revision_sha256: initial.revision_sha256,
            source: MILESTONE_SOURCE.into(),
        },
    )
    .unwrap_err();
    assert!(matches!(stale, MilestoneProgramUpdateError::Stale { .. }));

    for invalid in [
        "milestones 1.0\nmilestone boot { phase pre_input when }".to_string(),
        replacement.replace("milestone control", "milestone wrong_name"),
    ] {
        let error = update_milestone_program(
            &route,
            &root,
            &BrowserMilestoneProgramUpdateRequest {
                owner: String::new(),
                expected_revision_sha256: updated.revision_sha256.clone(),
                source: invalid,
            },
        )
        .unwrap_err();
        assert!(matches!(error, MilestoneProgramUpdateError::Invalid(_)));
        assert_eq!(
            fs::read_to_string(root.join("route.milestones")).unwrap(),
            replacement
        );
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn milestone_program_http_api_has_no_path_and_returns_conflict_for_stale_edits() {
    let root = temporary_root("milestone-http");
    fs::write(root.join("route.milestones"), MILESTONE_SOURCE).unwrap();
    fs::write(root.join("route.timeline"), milestone_timeline_source()).unwrap();
    let config = WorkbenchConfig {
        timeline_path: root.join("route.timeline"),
        repository_root: root.clone(),
        working_directory: root.clone(),
        game: root.join("unused-game"),
        dvd: root.join("unused-dvd"),
        state_root: root.join("state"),
    };
    let initial_revision = source_revision(MILESTONE_SOURCE.as_bytes());
    let smuggled_path = serde_json::json!({
        "expected_revision_sha256": initial_revision,
        "source": MILESTONE_SOURCE,
        "path": "../outside.milestones"
    });
    let response = call_http(
        &config,
        "POST",
        "/api/milestone-program",
        &serde_json::to_vec(&smuggled_path).unwrap(),
    );
    assert_eq!(response.status, 400);
    assert_eq!(
        fs::read_to_string(root.join("route.milestones")).unwrap(),
        MILESTONE_SOURCE
    );

    let replacement = MILESTONE_SOURCE.replace("stable 2", "stable 4");
    let request = BrowserMilestoneProgramUpdateRequest {
        owner: String::new(),
        expected_revision_sha256: initial_revision.clone(),
        source: replacement.clone(),
    };
    let response = call_http(
        &config,
        "POST",
        "/api/milestone-program",
        &serde_json::to_vec(&request).unwrap(),
    );
    assert_eq!(response.status, 200);
    let body: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(body["source"], replacement);
    assert!(body.get("path").is_none());

    let stale = call_http(
        &config,
        "POST",
        "/api/milestone-program",
        &serde_json::to_vec(&request).unwrap(),
    );
    assert_eq!(stale.status, 409);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn milestone_program_edit_rejects_symlink_escape() {
    use std::os::unix::fs::symlink;

    let root = temporary_root("milestone-symlink");
    let outside = temporary_root("milestone-symlink-outside");
    fs::write(outside.join("outside.milestones"), MILESTONE_SOURCE).unwrap();
    symlink(
        outside.join("outside.milestones"),
        root.join("route.milestones"),
    )
    .unwrap();
    let route = Timeline::parse(&milestone_timeline_source()).unwrap();
    let error = milestone_program_projection(&route, &root).unwrap_err();
    assert!(error.to_string().contains("symbolic link"));
    assert_eq!(
        fs::read_to_string(outside.join("outside.milestones")).unwrap(),
        MILESTONE_SOURCE
    );
    fs::remove_dir_all(root).unwrap();
    fs::remove_dir_all(outside).unwrap();
}

#[test]
fn materializes_segment_and_inclusive_segment_frame() {
    let root = temporary_root("materialize");
    write_tape(&root, "first.tape", &[1, 2, 3, 4]);
    write_tape(&root, "second.tape", &[5, 6, 7]);
    let route = timeline();
    let segment = materialize_lineage(
        &route,
        &root,
        "main",
        MaterializeTarget::ThroughSegment("boot_link.one".into()),
    )
    .unwrap();
    assert_eq!(segment.tape.frames.len(), 4);
    let scrubbed = materialize_lineage(
        &route,
        &root,
        "main",
        MaterializeTarget::ThroughSegmentFrame {
            segment: "link_exit.one".into(),
            frame: 0,
        },
    )
    .unwrap();
    assert_eq!(scrubbed.tape.frames.len(), 5);
    assert_eq!(scrubbed.tape.frames[4].pads[0].stick_x, 5);
    assert_eq!(scrubbed.steps[1].chain_start_frame, 4);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_selectors_off_lineage_and_artifact_escape() {
    let root = temporary_root("guardrails");
    write_tape(&root, "first.tape", &[1, 2, 3, 4]);
    write_tape(&root, "second.tape", &[5, 6, 7]);
    let error = materialize_lineage(
        &timeline(),
        &root,
        "main",
        MaterializeTarget::ThroughSegment("missing".into()),
    )
    .unwrap_err();
    assert!(error.to_string().contains("not on lineage"));
    assert!(checked_artifact_path(&root, Path::new("../outside.tape")).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn play_request_requires_unambiguous_scrub_target() {
    let request = PlayRequest {
        lineage: Some("main".into()),
        standalone_segment: None,
        through_segment: Some("boot_link".into()),
        segment: Some("boot_link".into()),
        frame: Some(1),
        takeover: true,
    };
    assert!(validate_play_request(&request).is_err());
}

#[test]
fn origin_policy_is_same_origin_or_non_browser() {
    let address: SocketAddr = "127.0.0.1:43123".parse().unwrap();
    assert!(origin_allowed(None, address));
    assert!(origin_allowed(Some("http://127.0.0.1:43123"), address));
    assert!(origin_allowed(Some("http://localhost:43123"), address));
    assert!(!origin_allowed(Some("https://hostile.example"), address));
}

#[test]
fn browser_ui_is_a_pannable_segment_graph_with_selection_details() {
    let html = include_str!("../../../assets/route_workbench.html");
    for required in [
        "aria-label=\"Route graph\"",
        "aria-label=\"Workspace\"",
        "id=\"projects\"",
        "id=\"tree\"",
        "id=\"detail\"",
        "graph-canvas",
        "graph-edges",
        "placeGraphNode",
        "bindGraphPan",
        "Detached / invalid",
        "grid-template-rows",
        "projectBootIcon",
        "🫠",
        "🗺️",
        "</span>${projectBootIcon(project)}<span class=\"project-label\">",
        "workspaceRoot=groups.some(group=>group.id==='routes')?'routes':null",
        "This predicate source belongs only to this goal",
        "data-capture-kind=\"project\"",
        "data-select-kind",
        "renderPlayableSegmentNode",
        "standaloneWorkspaceEntry",
        "id=\"workspaceNew\"",
        "id=\"workspaceNewTape\"",
        "id=\"workspaceNewFolder\"",
        "id=\"workspaceClone\"",
        "id=\"workspaceMove\"",
        "id=\"workspaceDelete\"",
        "/api/workspace/tapes/create",
        "/api/workspace/tapes/clone",
        "/api/workspace/folders/create",
        "/api/workspace/move",
        "/api/workspace/delete",
        "id=\"bootEditor\"",
        "/api/workspace/boot",
        "id=\"bootStageChoice\"",
        "id=\"bootRoomChoice\"",
        "id=\"bootCoordinateStatus\"",
        "bootCoordinateWarning",
        "adoptKnownRoom",
        "Saved override off",
        "Playback starts from tape",
        "Stored in tape · Not used",
        "room.spawn_points?.[0]??0",
        "id=\"bootTabInventory\"",
        "id=\"inventoryGrid\"",
        "inventory_items",
        "setInventoryEntry",
        "/api/workspace/stage-options",
        "projectOccurrence",
        "childSegments",
        "segment.parent==null",
        "segmentActions",
        "data-rename-segment",
        "renameSegment",
        "/api/segments/rename",
        "data-delete-segment",
        "Delete subtree",
        "deleteSegment",
        "/api/segments/delete/preview",
        "/api/segments/delete/apply",
        "data-delete-siblings",
        "Keep this; delete siblings",
        "deleteSiblings",
        "/api/segments/delete-siblings/preview",
        "/api/segments/delete-siblings/apply",
        "Checked-in sibling roots",
        "Direct draft sibling roots",
        "Generated search siblings",
        "remove every other displayed sibling",
        "The selected segment and its descendants are retained",
        "selection:{kind,id}",
        "const stop=segment?{kind:'segment',segment:id}",
        "goalDetail(segment.id",
        "segment.goal_proofs",
        "segment.option_visualization",
        "optionVisualization(segment)",
        "gameplayThumbnail(segment)",
        "Option execution overlay",
        "aria-label=\"Option intervals\"",
        "Consumed main-stick and camera curves",
        "overlay-marker target",
        "overlay-marker contact",
        "target_viewport",
        "goal_progress",
        "id=\"recordCountdown\"",
        "Child handoff",
        "id=\"recordingSpeed\"",
        "id=\"playbackSpeed\"",
        "speed_percent:speedPercent",
        "mode,speed_percent:speedPercent",
        "Resume (accelerated)",
        ">Playback</button>",
        ">Record child</button>",
        "window.localStorage",
        "countdown_seconds:countdownSeconds",
        "kind==='origin'?0",
        "data-capture-kind",
        "captureThumbnail",
        "/api/thumbnails/capture",
        "waitForThumbnail",
        "The node was left unchanged",
        "fetch(url,{cache:'no-store'})",
    ] {
        assert!(html.contains(required), "missing UI contract {required:?}");
    }
    for removed_dump in ["tree-icon", "Other roots"] {
        assert!(
            !html.contains(removed_dump),
            "legacy info-dump UI remains: {removed_dump:?}"
        );
    }
    assert!(!html.contains("data-expand"));
    assert!(!html.to_ascii_lowercase().contains("authored boot"));
    assert!(!html.contains("let collapsed = new Set()"));
    assert!(!html.contains("?ready=${Date.now()}"));
    assert!(html.contains("${segmentActions(segment)}</div>${goalDetail"));
    assert!(!html.contains("${segmentActions(segment)}</section>"));
}

#[test]
fn thumbnail_cache_is_content_addressed_validated_and_path_safe() {
    let root = temporary_root("thumbnail-cache");
    write_tape(&root, "first.tape", &[1, 2, 3, 4]);
    write_tape(&root, "second.tape", &[5, 6, 7]);
    fs::write(root.join("route.timeline"), b"timeline thumbnail-test\n").unwrap();
    let state_root = root.join("state");
    let game = root.join("game.exe");
    fs::create_dir(&state_root).unwrap();
    fs::write(&game, b"build-one").unwrap();
    let config = WorkbenchConfig {
        timeline_path: root.join("route.timeline"),
        repository_root: root.clone(),
        working_directory: root.clone(),
        game: game.clone(),
        dvd: root.join("disc.iso"),
        state_root: state_root.clone(),
    };

    let key = thumbnail_key("segment", "boundary-a");
    assert_eq!(key.len(), 64);
    assert_eq!(key, thumbnail_key("segment", "boundary-a"));
    assert_ne!(key, thumbnail_key("segment", "boundary-b"));
    assert_ne!(key, thumbnail_key("draft", "boundary-a"));

    fs::write(&game, b"a completely different game build").unwrap();
    assert_eq!(
        key,
        thumbnail_key("segment", "boundary-a"),
        "rebuilding the executable must not invalidate an illustrative thumbnail"
    );
    assert_eq!(
        key,
        thumbnail_key("segment", "boundary-a"),
        "renaming a segment must not invalidate its terminal-state thumbnail"
    );

    let thumbnail_root = state_root.join(THUMBNAIL_DIRECTORY);
    fs::create_dir(&thumbnail_root).unwrap();
    let path = thumbnail_cache_path(&state_root, &key);
    fs::write(&path, b"not a png").unwrap();
    assert!(!thumbnail_file_is_valid(&path));
    assert_eq!(
        thumbnail_response(&config, &thumbnail_url(&key)).status,
        404
    );

    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend_from_slice(b"terminal-frame");
    fs::write(&path, &png).unwrap();
    assert!(thumbnail_file_is_valid(&path));
    let response = thumbnail_response(&config, &thumbnail_url(&key));
    assert_eq!(response.status, 200);
    assert_eq!(response.content_type, "image/png");
    assert_eq!(response.body, png);
    assert_eq!(
        thumbnail_response(&config, "/api/thumbnails/../secret.png").status,
        404
    );
    assert_eq!(
        thumbnail_response(&config, "/api/thumbnails/not-a-digest.png").status,
        404
    );

    let selection = BrowserSelection::Segment {
        id: "boot_link.one".into(),
    };
    let prepared = prepare_missing_playback_thumbnail(&timeline(), &config, &selection)
        .unwrap()
        .expect("a missing thumbnail should be prepared for normal playback");
    assert!(
        prepared
            .path
            .starts_with(state_root.join(THUMBNAIL_DIRECTORY))
    );
    assert_eq!(
        prepared.url,
        thumbnail_url(prepared.path.file_stem().unwrap().to_str().unwrap())
    );
    fs::write(&prepared.path, &png).unwrap();
    assert!(
        prepare_missing_playback_thumbnail(&timeline(), &config, &selection)
            .unwrap()
            .is_none(),
        "normal playback must not overwrite an existing valid thumbnail"
    );

    fs::create_dir_all(root.join("routes/qa")).unwrap();
    fs::write(
        root.join("routes/qa/canary.tape"),
        InputTape::default().encode().unwrap(),
    )
    .unwrap();
    let project_selection = BrowserSelection::Project {
        id: "routes/qa/canary".into(),
    };
    let project_thumbnail =
        prepare_missing_playback_thumbnail(&timeline(), &config, &project_selection)
            .unwrap()
            .expect("standalone tapes use the same thumbnail preparation path");
    assert!(
        project_thumbnail
            .path
            .starts_with(state_root.join(THUMBNAIL_DIRECTORY))
    );

    let graph = graph_from_timeline(&timeline(), &root).unwrap();
    let reachable_key = graph_node_thumbnail_key(&graph, &selection).unwrap();
    let reachable_path = thumbnail_cache_path(&state_root, &reachable_key);
    if !reachable_path.exists() {
        fs::write(&reachable_path, &png).unwrap();
    }
    let orphan_key = "f".repeat(64);
    let orphan_path = thumbnail_cache_path(&state_root, &orphan_key);
    fs::write(&orphan_path, &png).unwrap();
    let unrelated_path = state_root.join(THUMBNAIL_DIRECTORY).join("README.txt");
    fs::write(&unrelated_path, b"not managed by the PNG cache").unwrap();
    let preview = prune_orphaned_thumbnails(&graph, &state_root, false).unwrap();
    assert_eq!(preview.orphaned.len(), 2);
    assert_eq!(preview.moved, 0);
    assert!(orphan_path.exists());
    let applied = prune_orphaned_thumbnails(&graph, &state_root, true).unwrap();
    assert_eq!(applied.moved, 2);
    assert!(reachable_path.exists());
    assert!(!orphan_path.exists());
    assert!(
        applied
            .trash_transaction
            .unwrap()
            .join(format!("{orphan_key}.png"))
            .exists()
    );
    assert!(unrelated_path.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn browser_record_countdown_defaults_to_three_and_rejects_out_of_range_values() {
    let defaulted = serde_json::from_str::<BrowserRecordRequest>(
        r#"{"parent":{"kind":"draft","id":"draft-one"},"label":"child"}"#,
    )
    .unwrap();
    assert_eq!(
        defaulted.countdown_seconds,
        DEFAULT_RECORD_INPUT_COUNTDOWN_SECONDS
    );

    for seconds in [0, MAX_RECORD_INPUT_COUNTDOWN_SECONDS] {
        let request = serde_json::from_value::<BrowserRecordRequest>(serde_json::json!({
            "parent": {"kind": "draft", "id": "draft-one"},
            "label": "child",
            "countdown_seconds": seconds,
        }))
        .unwrap();
        assert_eq!(request.countdown_seconds, seconds);
    }

    let error = serde_json::from_value::<BrowserRecordRequest>(serde_json::json!({
        "parent": {"kind": "draft", "id": "draft-one"},
        "countdown_seconds": MAX_RECORD_INPUT_COUNTDOWN_SECONDS + 1,
    }))
    .unwrap_err();
    assert!(error.to_string().contains("between 0 and 10 seconds"));
}

#[test]
fn browser_speed_settings_default_and_reject_unlisted_values() {
    let playback = serde_json::from_str::<BrowserPlayRequest>(
            r#"{"selection":{"kind":"segment","id":"one"},"stop":{"kind":"segment","segment":"one"},"handoff":true}"#,
        )
        .unwrap();
    assert_eq!(playback.speed_percent, 100);
    assert_eq!(playback.mode, PlaybackMode::Playback);

    for speed_percent in PLAYBACK_SPEED_PERCENTAGES {
        let request = serde_json::from_value::<BrowserPlayRequest>(serde_json::json!({
            "selection": {"kind": "segment", "id": "one"},
            "stop": {"kind": "segment", "segment": "one"},
            "handoff": true,
            "speed_percent": speed_percent,
        }));
        assert!(request.is_ok(), "playback speed {speed_percent}");
    }
    assert!(
        serde_json::from_value::<BrowserPlayRequest>(serde_json::json!({
            "selection": {"kind": "segment", "id": "one"},
            "stop": {"kind": "segment", "segment": "one"},
            "handoff": true,
            "speed_percent": 201,
        }))
        .unwrap_err()
        .to_string()
        .contains("playback speed percentage 201 is not supported")
    );

    for speed_percent in RECORDING_SPEED_PERCENTAGES {
        let request = serde_json::from_value::<BrowserRecordRequest>(serde_json::json!({
            "parent": {"kind": "draft", "id": "one"},
            "speed_percent": speed_percent,
        }));
        assert!(request.is_ok(), "recording speed {speed_percent}");
    }
    assert!(
        serde_json::from_value::<BrowserRecordRequest>(serde_json::json!({
            "parent": {"kind": "draft", "id": "one"},
            "speed_percent": 0,
        }))
        .unwrap_err()
        .to_string()
        .contains("recording speed percentage 0 is not supported")
    );
}

#[test]
fn recording_speed_is_fixed_step_host_pacing_not_a_tape_rate() {
    let mut command = Command::new("game");
    append_fixed_step_pacing(&mut command, 50);
    let arguments = command
        .get_args()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert_eq!(
        arguments,
        ["--fixed-step", "--fixed-step-speed-percent", "50"]
    );
}

#[test]
fn browser_accepts_playback_and_accelerated_resume() {
    let playback = BrowserPlayRequest {
        selection: BrowserSelection::Segment {
            id: "link_exit.one".into(),
        },
        stop: BrowserStop::Tick { tick: 1 },
        handoff: true,
        mode: PlaybackMode::Playback,
        speed_percent: 100,
    };
    assert!(validate_playback_origin(&playback).is_ok());

    let accelerated = BrowserPlayRequest {
        selection: BrowserSelection::Segment {
            id: "link_exit.one".into(),
        },
        stop: BrowserStop::Tick { tick: 1 },
        handoff: true,
        mode: PlaybackMode::ResumeAccelerated,
        speed_percent: 100,
    };
    assert!(validate_playback_origin(&accelerated).is_ok());

    let no_handoff = BrowserPlayRequest {
        handoff: false,
        ..accelerated
    };
    assert!(validate_playback_origin(&no_handoff).is_err());
}

#[test]
fn milestone_program_update_writes_only_the_selected_goal_source() {
    let root = temporary_root("owned-milestone-update");
    let first = "milestones 1.0\nmilestone first {\n  phase post_sim\n  when player.exists\n}\n";
    let second = "milestones 1.0\nmilestone second {\n  phase post_sim\n  when event.running\n}\n";
    fs::write(root.join("first.milestones"), first).unwrap();
    fs::write(root.join("second.milestones"), second).unwrap();
    let route = Timeline::parse(
        r#"timeline owned
segment root root profile boot_to_fsp103 uses baseline boot_to_fsp103 starts clean produces one
segment child after root profile fsp103_to_fsp104 uses baseline fsp103_to_fsp104 starts one produces two
goal first_goal on root predicate first source first.milestones
goal second_goal on child predicate second source second.milestones
continuation main starts root@clean
continue main with root after root@clean
continue main with child after root@one
"#,
    )
    .unwrap();
    let projected = goal_predicate_program_projection(&route, &root, "first_goal").unwrap();
    let replacement = first.replace("when player.exists", "stable 2\n  when player.exists");
    update_milestone_program(
        &route,
        &root,
        &BrowserMilestoneProgramUpdateRequest {
            owner: "first_goal".into(),
            expected_revision_sha256: projected.revision_sha256,
            source: replacement.clone(),
        },
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(root.join("first.milestones")).unwrap(),
        replacement
    );
    assert_eq!(
        fs::read_to_string(root.join("second.milestones")).unwrap(),
        second
    );

    let current = source_revision(replacement.as_bytes());
    let coupled = format!("{replacement}\n{second}");
    assert!(
        update_milestone_program(
            &route,
            &root,
            &BrowserMilestoneProgramUpdateRequest {
                owner: "first_goal".into(),
                expected_revision_sha256: current,
                source: coupled,
            },
        )
        .is_err()
    );
    assert_eq!(
        fs::read_to_string(root.join("second.milestones")).unwrap(),
        second
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn accelerated_resume_hides_the_complete_boot_rooted_tape_even_for_root_nodes() {
    assert_eq!(
        playback_fast_forward_frames(
            PlaybackSettings {
                speed_percent: 0,
                fast: true,
            },
            439,
        ),
        Some(439)
    );
    assert_eq!(
        playback_fast_forward_frames(
            PlaybackSettings {
                speed_percent: 100,
                fast: false,
            },
            439,
        ),
        None
    );
}

#[test]
fn compiles_checked_in_tas_artifacts() {
    let root = temporary_root("tas");
    fs::write(
        root.join("boot.tas"),
        "dusktape 1\nrate 30/1\nports 0x0f\nstate neutral {}\nframe neutral\n",
    )
    .unwrap();
    let route = Timeline::parse(
        r#"
timeline tas
segment boot_link.tas root profile boot_to_fsp103 uses tas boot.tas starts clean produces control
continuation main starts root@clean
continue main with boot_link.tas after root@clean
"#,
    )
    .unwrap();
    let tape = load_segment_tape(&route.segments["boot_link.tas"], &root).unwrap();
    assert_eq!(tape.frames.len(), 1);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn checked_in_intro_exposes_native_reproved_predicate_anchor() {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../..")
        .canonicalize()
        .unwrap();
    let timeline_path = repository.join("routes/intro.timeline");
    let route = load_authoritative_timeline(&timeline_path).unwrap();
    let graph = graph_from_timeline(&route, timeline_path.parent().unwrap()).unwrap();
    assert!(graph.predicate_program.is_none());
    assert_eq!(graph.goals.len(), 2);
    assert!(graph.goals.iter().all(|goal| {
        goal.predicate_program.definitions.len() == 1
            && goal.predicate_program.definitions[0].name == goal.predicate
    }));
    assert_ne!(
        graph.goals[0].predicate_program.program_sha256,
        graph.goals[1].predicate_program.program_sha256
    );
    assert_eq!(
        graph
            .segments
            .iter()
            .find(|segment| segment.id == "to_ordon_spring_q129")
            .and_then(|segment| segment.parent.as_deref()),
        Some("golf439")
    );
    assert!(graph.goals.iter().any(|goal| {
        goal.id == "link_control" && goal.segment == "golf439" && goal.predicate == "link_control"
    }));
    let segment = graph
        .segments
        .iter()
        .find(|segment| segment.id == "golf439")
        .unwrap();
    assert!(segment.playable);
    assert!(segment.recordable);
    assert_eq!(segment.predicate_proof, "verified");
    assert_eq!(segment.goal_proofs.len(), 1);
    assert_eq!(segment.goal_proofs[0].goal, "link_control");
    assert_eq!(segment.record_anchors.len(), 1);
    let continuation = graph
        .segments
        .iter()
        .find(|segment| segment.id == "to_ordon_spring_q129")
        .unwrap();
    assert!(continuation.playable);
    assert!(continuation.recordable);
    assert_eq!(continuation.predicate_proof, "verified");
    assert_eq!(continuation.first_hit_tick, Some(129));
    assert_eq!(continuation.goal_proofs.len(), 1);
    assert_eq!(
        continuation.goal_proofs[0].goal,
        "ordon_spring_load_committed"
    );
    assert_eq!(continuation.record_anchors.len(), 1);
    let boot = graph.origin.as_ref().unwrap();
    assert!(boot.recordable_from_boot);
    assert_eq!(boot.id, "boot");
}

#[test]
fn checked_in_ordon_spring_incumbent_composes_its_exact_boot_prefix() {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../..")
        .canonicalize()
        .unwrap();
    let timeline_path = repository.join("routes/intro.timeline");
    let route = load_authoritative_timeline(&timeline_path).unwrap();
    let artifact_root = timeline_path.parent().unwrap();
    let graph = graph_from_timeline(&route, artifact_root).unwrap();
    let prefix = materialize_lineage(
        &route,
        artifact_root,
        "main",
        MaterializeTarget::ThroughSegment("golf439".into()),
    )
    .unwrap();
    assert_eq!(prefix.tape.frames.len(), 440);

    let (segment_id, expected_output) =
        ("to_ordon_spring_q129", "59ba70a093e63bc298d72f40a7ea21bc");
    let segment = &route.segments[segment_id];
    assert_eq!(segment.end_fingerprint, expected_output);
    let card = graph
        .segments
        .iter()
        .find(|candidate| candidate.id == segment_id)
        .unwrap();
    assert!(card.playable);
    assert_eq!(card.parent.as_deref(), Some("golf439"));
    let continuation = load_segment_tape(segment, artifact_root).unwrap();
    assert_eq!(continuation.frames.len(), 130);
    let playback = materialize_segment_playback(&route, artifact_root, segment_id, None).unwrap();
    assert_eq!(playback.tape.frames.len(), 570);
    assert_eq!(
        segment_parent_frame_count(
            &route,
            artifact_root,
            segment.parent.as_deref(),
            &playback.tape,
            segment_id,
        )
        .unwrap(),
        440
    );
    assert_eq!(playback.lineage, None);
    assert_eq!(playback.segment.as_deref(), Some(segment_id));
    assert_eq!(
        &playback.tape.frames[..prefix.tape.frames.len()],
        prefix.tape.frames.as_slice()
    );
    assert_eq!(
        &playback.tape.frames[prefix.tape.frames.len()..],
        continuation.frames.as_slice()
    );
    let first_local_frame =
        materialize_segment_playback(&route, artifact_root, segment_id, Some(0)).unwrap();
    assert_eq!(first_local_frame.tape.frames.len(), 441);
    assert_eq!(
        first_local_frame.tape.frames.last(),
        continuation.frames.first()
    );
    let root_playback =
        materialize_segment_playback(&route, artifact_root, "golf439", None).unwrap();
    assert!(
        segment_parent_frame_count(&route, artifact_root, None, &root_playback.tape, "golf439",)
            .is_err()
    );
    let mut tampered = playback.tape.clone();
    tampered.frames[0].pads[0].stick_x = tampered.frames[0].pads[0].stick_x.wrapping_add(1);
    assert!(
        segment_parent_frame_count(
            &route,
            artifact_root,
            segment.parent.as_deref(),
            &tampered,
            segment_id,
        )
        .is_err()
    );
    let sibling_request = r#"{
            "selection":{"kind":"segment","id":"another_segment"},
            "stop":{"kind":"segment","segment":"another_segment"},
            "handoff":true,
            "mode":"playback"
        }"#;
    assert!(serde_json::from_str::<BrowserPlayRequest>(sibling_request).is_ok());
}

#[test]
fn authored_boot_recording_status_becomes_a_proved_root_draft() {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../..")
        .canonicalize()
        .unwrap();
    let timeline_path = repository.join("routes/intro.timeline");
    let route = load_authoritative_timeline(&timeline_path).unwrap();
    let artifact_root = timeline_path.parent().unwrap();
    let program = origin_predicate_program_projection(&route, artifact_root)
        .unwrap()
        .unwrap();
    let definition = program
        .definitions
        .iter()
        .find(|definition| definition.name == "process_boot")
        .unwrap();
    assert!(is_exact_boot_boundary_predicate(definition));

    let state = temporary_root("boot-root-draft");
    let id = "draft-boot-root";
    let directory = drafts_root(&state).unwrap().join(id);
    fs::create_dir(&directory).unwrap();
    let continuation = InputTape {
        frames: vec![InputFrame {
            owned_ports: 0x0f,
            pads: [RawPadState::default(); 4],
            ..InputFrame::default()
        }],
        ..InputTape::default()
    };
    fs::write(directory.join(DRAFT_TAPE), continuation.encode().unwrap()).unwrap();
    let empty = InputTape::default();
    fs::write(directory.join("playback.tape"), empty.encode().unwrap()).unwrap();
    let mut manifest = DraftManifest {
        schema: DRAFT_SCHEMA.into(),
        id: id.into(),
        label: "Boot root".into(),
        parent: DraftParent::Milestone {
            id: "process_boot".into(),
            program_sha256: program.program_sha256.clone(),
            definition_sha256: definition.definition_sha256.clone(),
            boundary_fingerprint: None,
        },
        parent_tape_sha256: tape_digest(&empty).unwrap(),
        created_unix_ms: 1,
        session_token: "00112233445566778899aabbccddeeff".into(),
        expected_start_milestone: Some("process_boot".into()),
        expected_start_fingerprint: None,
        tape: DRAFT_TAPE.into(),
        status: DraftStatus::Recording,
        endpoint_kind: "manual_stop".into(),
        verification: "unverified".into(),
        start_boundary_verified: false,
        accelerated_parent_replay: false,
        parent_frames: 0,
        tape_sha256: None,
        tape_bytes: None,
        result_tape_sha256: None,
        frames: None,
        error: None,
    };
    let fingerprint = "0123456789abcdef0123456789abcdef";
    let mut status = serde_json::json!({
        "schema": "dusklight.input-recording/v2",
        "status": "success",
        "tape": fs::canonicalize(directory.join(DRAFT_TAPE)).unwrap(),
        "frame_count": 1,
        "frame_capacity": 1080000,
        "handoff_reached": true,
        "capacity_exhausted": false,
        "error": null,
        "process_success": true,
        "session_token": manifest.session_token,
        "start_milestone": "process_boot",
        "start_fingerprint": fingerprint,
        "expected_start_fingerprint": null,
        "start_boundary_kind": "boot",
        "start_boundary_index": 0,
        "start_program_digest": program.program_sha256,
        "start_definition_digest": definition.definition_sha256,
        "start_tape_frame": null
    });
    let status_path = directory.join(format!("{DRAFT_TAPE}.status.json"));
    status["start_boundary_index"] = serde_json::json!(1);
    fs::write(&status_path, serde_json::to_vec(&status).unwrap()).unwrap();
    let mut rejected = manifest.clone();
    finalize_recording(&directory, &mut rejected, Some(true));
    assert_eq!(rejected.status, DraftStatus::ProcessFailure);

    status["start_boundary_index"] = serde_json::json!(0);
    fs::write(&status_path, serde_json::to_vec(&status).unwrap()).unwrap();
    finalize_recording(&directory, &mut manifest, Some(true));
    assert_eq!(manifest.status, DraftStatus::Ready);
    assert!(manifest.start_boundary_verified);
    assert!(matches!(
        &manifest.parent,
        DraftParent::Milestone {
            boundary_fingerprint: Some(actual),
            ..
        } if actual == fingerprint
    ));
    write_draft_manifest(&directory, &manifest, true).unwrap();
    let materialized = materialize_draft(&route, artifact_root, &state, id).unwrap();
    assert_eq!(materialized.tape.frames.len(), 1);
    assert!(
        draft_parent_frame_count(&route, artifact_root, &state, id, 1).is_err(),
        "there is no meaningful parent-origin playback before Boot"
    );
    fs::remove_dir_all(state).unwrap();
}

fn install_ready_draft(
    repository_root: &Path,
    state_root: &Path,
    id: &str,
    values: &[i8],
) -> DraftManifest {
    let route = timeline();
    let parent = materialize_lineage(
        &route,
        repository_root,
        "main",
        MaterializeTarget::ThroughSegment("link_exit.one".into()),
    )
    .unwrap()
    .tape;
    let continuation = InputTape {
        frames: values
            .iter()
            .map(|value| InputFrame {
                owned_ports: 0x0f,
                pads: [
                    RawPadState {
                        stick_x: *value,
                        ..RawPadState::default()
                    },
                    RawPadState::default(),
                    RawPadState::default(),
                    RawPadState::default(),
                ],
                ..InputFrame::default()
            })
            .collect(),
        ..InputTape::default()
    };
    let continuation_bytes = continuation.encode().unwrap();
    let result = concatenate(vec![
        ChainSegment::all(parent.clone()),
        ChainSegment::all(continuation),
    ])
    .unwrap()
    .tape;
    let directory = drafts_root(state_root).unwrap().join(id);
    fs::create_dir(&directory).unwrap();
    fs::write(directory.join(DRAFT_TAPE), &continuation_bytes).unwrap();
    fs::write(directory.join("playback.tape"), parent.encode().unwrap()).unwrap();
    let manifest = DraftManifest {
        schema: DRAFT_SCHEMA.into(),
        id: id.into(),
        label: "Test branch".into(),
        parent: DraftParent::Segment {
            id: "link_exit.one".into(),
            terminal_milestone: "exit".into(),
            boundary_fingerprint: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
        },
        parent_tape_sha256: tape_digest(&parent).unwrap(),
        created_unix_ms: 1,
        session_token: "00112233445566778899aabbccddeeff".into(),
        expected_start_milestone: Some("entered-f-sp104".into()),
        expected_start_fingerprint: Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()),
        tape: DRAFT_TAPE.into(),
        status: DraftStatus::Ready,
        endpoint_kind: "manual_stop".into(),
        verification: "unverified".into(),
        start_boundary_verified: true,
        accelerated_parent_replay: false,
        parent_frames: parent.frames.len() as u64,
        tape_sha256: Some(format!("{:x}", Sha256::digest(&continuation_bytes))),
        tape_bytes: Some(continuation_bytes.len() as u64),
        result_tape_sha256: Some(tape_digest(&result).unwrap()),
        frames: Some(values.len() as u64),
        error: None,
    };
    write_draft_manifest(&directory, &manifest, true).unwrap();
    manifest
}

#[test]
fn successful_human_recording_installs_its_terminal_thumbnail_once() {
    let root = temporary_root("recording-thumbnail");
    write_tape(&root, "first.tape", &[1, 2, 3, 4]);
    write_tape(&root, "second.tape", &[5, 6, 7]);
    let state = root.join("state");
    let id = "draft-recording-thumbnail";
    let manifest = install_ready_draft(&root, &state, id, &[8, 9]);
    let directory = state.join("drafts").join(id);
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend_from_slice(b"retained-terminal-frame");
    fs::write(directory.join(DRAFT_TERMINAL_THUMBNAIL), &png).unwrap();
    let game = root.join("game.exe");
    fs::write(&game, b"game-build").unwrap();
    let config = WorkbenchConfig {
        timeline_path: root.join("route.timeline"),
        repository_root: root.clone(),
        working_directory: root.clone(),
        game: game.clone(),
        dvd: root.join("disc.iso"),
        state_root: state.clone(),
    };

    install_recording_thumbnail(&directory, &manifest, &config).unwrap();
    assert!(!directory.join(DRAFT_TERMINAL_THUMBNAIL).exists());
    let key = thumbnail_key("draft", manifest.result_tape_sha256.as_deref().unwrap());
    assert_eq!(fs::read(thumbnail_cache_path(&state, &key)).unwrap(), png);
    let digest = crate::artifact::Digest(Sha256::digest(png).into());
    assert!(
        ContentStore::initialize(state.join("content"))
            .unwrap()
            .blob_path(digest)
            .is_file()
    );

    let mut graph = graph_with_drafts(&timeline(), &root, &state).unwrap();
    decorate_graph_thumbnails(&mut graph, &config).unwrap();
    assert_eq!(
        graph.drafts[0].thumbnail.as_deref(),
        Some(thumbnail_url(&key).as_str())
    );
    fs::remove_dir_all(root).unwrap();
}

fn write_success_status(directory: &Path, manifest: &DraftManifest, frame_count: u64) {
    let status = serde_json::json!({
        "schema": "dusklight.input-recording/v2",
        "status": "success",
        "tape": fs::canonicalize(directory.join(DRAFT_TAPE)).unwrap(),
        "frame_count": frame_count,
        "frame_capacity": 1080000,
        "handoff_reached": true,
        "capacity_exhausted": false,
        "error": null,
        "process_success": true,
        "session_token": manifest.session_token,
        "start_milestone": manifest.expected_start_milestone,
        "start_fingerprint": manifest.expected_start_fingerprint,
        "expected_start_fingerprint": manifest.expected_start_fingerprint,
        "start_boundary_kind": "tick",
        "start_boundary_index": null,
        "start_program_digest": null,
        "start_definition_digest": null,
        "start_tape_frame": manifest.parent_frames - 1
    });
    fs::write(
        directory.join(format!("{DRAFT_TAPE}.status.json")),
        serde_json::to_vec(&status).unwrap(),
    )
    .unwrap();
}

#[test]
fn draft_suffix_composes_after_exact_two_segment_lineage() {
    let root = temporary_root("draft-chain");
    write_tape(&root, "first.tape", &[1, 2, 3, 4]);
    write_tape(&root, "second.tape", &[5, 6, 7]);
    let state = root.join("state");
    let id = "draft-test-chain";
    install_ready_draft(&root, &state, id, &[8, 9]);
    let materialized = materialize_draft(&timeline(), &root, &state, id).unwrap();
    assert_eq!(
        materialized
            .tape
            .frames
            .iter()
            .map(|frame| frame.pads[0].stick_x)
            .collect::<Vec<_>>(),
        [1, 2, 3, 4, 5, 6, 7, 8, 9]
    );
    let graph = graph_with_drafts(&timeline(), &root, &state).unwrap();
    assert!(graph.drafts[0].playable);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn direct_and_nested_draft_parent_origins_use_exact_cli_boundary() {
    let root = temporary_root("draft-parent-origin");
    write_tape(&root, "first.tape", &[1, 2, 3, 4]);
    write_tape(&root, "second.tape", &[5, 6, 7]);
    let state = root.join("state");
    let direct_id = "draft-parent-direct";
    let mut direct_manifest = install_ready_draft(&root, &state, direct_id, &[8, 9]);
    let direct = materialize_draft(&timeline(), &root, &state, direct_id).unwrap();
    assert_eq!(direct.tape.frames.len(), 9);
    assert_eq!(
        draft_parent_frame_count(&timeline(), &root, &state, direct_id, 9).unwrap(),
        7
    );
    assert!(validate_parent_boundary(0, 2, 2).is_err());
    assert!(validate_parent_boundary(7, 0, 7).is_err());
    assert!(validate_parent_boundary(7, 2, 7).is_err());
    assert!(validate_parent_boundary(7, 1, 9).is_err());
    assert!(
        validate_parent_boundary_metadata(440, 106, 439, Some(107), 546).is_err(),
        "compensating manifest corruption must not reveal one frame early"
    );
    assert!(validate_parent_boundary_metadata(440, 106, 440, Some(106), 546).is_ok());

    let direct_directory = drafts_root(&state).unwrap().join(direct_id);
    direct_manifest.parent_frames = 6;
    direct_manifest.frames = Some(3);
    fs::remove_file(direct_directory.join(DRAFT_FINAL_MANIFEST)).unwrap();
    write_draft_manifest(&direct_directory, &direct_manifest, true).unwrap();
    assert!(
        draft_parent_frame_count(&timeline(), &root, &state, direct_id, 9).is_err(),
        "compensating corruption in a real manifest must not move the decoded boundary"
    );
    assert!(materialize_draft(&timeline(), &root, &state, direct_id).is_err());
    assert!(
        !graph_with_drafts(&timeline(), &root, &state)
            .unwrap()
            .drafts
            .into_iter()
            .find(|draft| draft.id == direct_id)
            .unwrap()
            .playable,
        "compensating frame corruption must make the draft structurally unplayable"
    );
    direct_manifest.parent_frames = 7;
    direct_manifest.frames = Some(2);
    fs::remove_file(direct_directory.join(DRAFT_FINAL_MANIFEST)).unwrap();
    write_draft_manifest(&direct_directory, &direct_manifest, true).unwrap();

    let nested_id = "draft-parent-nested";
    let mut nested = install_ready_draft(&root, &state, nested_id, &[10, 11]);
    let nested_directory = drafts_root(&state).unwrap().join(nested_id);
    let (_, nested_continuation) = read_draft_tape(&nested_directory).unwrap();
    let nested_result = concatenate(vec![
        ChainSegment::all(direct.tape.clone()),
        ChainSegment::all(nested_continuation),
    ])
    .unwrap()
    .tape;
    nested.parent = DraftParent::Draft {
        id: direct_id.into(),
        parent_tape_sha256: tape_digest(&direct.tape).unwrap(),
    };
    nested.parent_tape_sha256 = tape_digest(&direct.tape).unwrap();
    nested.parent_frames = 9;
    nested.expected_start_milestone = None;
    nested.expected_start_fingerprint = None;
    nested.start_boundary_verified = false;
    nested.result_tape_sha256 = Some(tape_digest(&nested_result).unwrap());
    fs::remove_file(nested_directory.join(DRAFT_FINAL_MANIFEST)).unwrap();
    write_draft_manifest(&nested_directory, &nested, true).unwrap();

    let nested_full = materialize_draft(&timeline(), &root, &state, nested_id).unwrap();
    assert_eq!(nested_full.tape.frames.len(), 11);
    assert_eq!(
        draft_parent_frame_count(&timeline(), &root, &state, nested_id, 11).unwrap(),
        9
    );

    let mut command = Command::new("game");
    append_playback_args(
        &mut command,
        Path::new("disc.iso"),
        Path::new("full-chain.tape"),
        "release",
        Path::new("state"),
        PlaybackCliOptions {
            seed_stage: None,
            fast_forward_frames: Some(9),
            playback: PlaybackSettings {
                speed_percent: 0,
                fast: true,
            },
        },
    );
    let arguments = command
        .get_args()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let flag = arguments
        .iter()
        .position(|argument| argument == "--input-tape-fast-forward-frames")
        .unwrap();
    assert_eq!(arguments[flag + 1], "9");
    assert!(
        !arguments
            .iter()
            .any(|argument| argument == "--input-tape-fast-forward-visible")
    );
    assert_eq!(
        arguments
            .windows(2)
            .find(|window| window[0] == "--fixed-step-speed-percent")
            .unwrap()[1],
        "0"
    );
    assert_eq!(
        arguments
            .windows(2)
            .find(|window| window[0] == "--input-tape")
            .unwrap()[1],
        "full-chain.tape"
    );
    assert_eq!(
        arguments
            .windows(2)
            .find(|window| window[0] == "--renderer-cache-root")
            .unwrap()[1],
        "renderer-cache"
    );

    let mut boot = Command::new("game");
    append_playback_args(
        &mut boot,
        Path::new("disc.iso"),
        Path::new("full-chain.tape"),
        "release",
        Path::new("state"),
        PlaybackCliOptions {
            seed_stage: None,
            fast_forward_frames: None,
            playback: PlaybackSettings {
                speed_percent: 100,
                fast: false,
            },
        },
    );
    assert!(
        !boot
            .get_args()
            .any(|argument| argument == "--input-tape-fast-forward-frames")
    );

    let mut recording = Command::new("game");
    append_accelerated_recording_prefix(
        &mut recording,
        Path::new("playback.tape"),
        nested.parent_frames as usize,
        3,
    );
    let recording_arguments = recording
        .get_args()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert_eq!(
        recording_arguments,
        [
            "--input-tape",
            "playback.tape",
            "--input-tape-end",
            "release",
            "--input-tape-fast-forward-frames",
            "9",
            "--record-input-countdown-seconds",
            "3"
        ]
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn accelerated_unnamed_parent_requires_exact_native_tape_end_boundary() {
    let root = temporary_root("draft-accelerated-boundary");
    write_tape(&root, "first.tape", &[1, 2, 3, 4]);
    write_tape(&root, "second.tape", &[5, 6, 7]);
    let state = root.join("state");
    let id = "draft-accelerated";
    let mut manifest = install_ready_draft(&root, &state, id, &[8, 9]);
    let directory = drafts_root(&state).unwrap().join(id);
    manifest.status = DraftStatus::Recording;
    manifest.expected_start_milestone = None;
    manifest.expected_start_fingerprint = None;
    manifest.start_boundary_verified = false;
    manifest.accelerated_parent_replay = true;
    write_success_status(&directory, &manifest, 2);
    let status_path = directory.join(format!("{DRAFT_TAPE}.status.json"));
    let mut status: serde_json::Value =
        serde_json::from_slice(&fs::read(&status_path).unwrap()).unwrap();
    status["start_boundary_index"] = manifest.parent_frames.into();
    fs::write(&status_path, serde_json::to_vec(&status).unwrap()).unwrap();

    let mut exact = manifest.clone();
    finalize_recording(&directory, &mut exact, Some(true));
    assert_eq!(exact.status, DraftStatus::Ready);
    assert!(!exact.start_boundary_verified);

    status["start_boundary_index"] = (manifest.parent_frames - 1).into();
    fs::write(&status_path, serde_json::to_vec(&status).unwrap()).unwrap();
    let mut early = manifest;
    finalize_recording(&directory, &mut early, Some(true));
    assert_eq!(early.status, DraftStatus::ProcessFailure);
    assert_eq!(
        early.error.as_deref(),
        Some("native recording status contradicts process result")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn tampered_continuation_is_neither_playable_nor_loadable() {
    let root = temporary_root("draft-tamper");
    write_tape(&root, "first.tape", &[1, 2, 3, 4]);
    write_tape(&root, "second.tape", &[5, 6, 7]);
    let state = root.join("state");
    let id = "draft-test-tamper";
    install_ready_draft(&root, &state, id, &[8, 9]);
    let tape = drafts_root(&state).unwrap().join(id).join(DRAFT_TAPE);
    let mut bytes = fs::read(&tape).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    fs::write(&tape, bytes).unwrap();
    let graph = graph_with_drafts(&timeline(), &root, &state).unwrap();
    assert!(!graph.drafts[0].playable);
    assert!(
        graph.drafts[0]
            .error
            .as_deref()
            .unwrap()
            .contains("tampered")
    );
    assert!(materialize_draft(&timeline(), &root, &state, id).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn capacity_exhausted_draft_is_visible_but_not_branchable() {
    let root = temporary_root("draft-capacity");
    write_tape(&root, "first.tape", &[1, 2, 3, 4]);
    write_tape(&root, "second.tape", &[5, 6, 7]);
    let state = root.join("state");
    let id = "draft-test-capacity";
    let mut manifest = install_ready_draft(&root, &state, id, &[8]);
    manifest.status = DraftStatus::CapacityExhausted;
    let directory = drafts_root(&state).unwrap().join(id);
    fs::remove_file(directory.join(DRAFT_FINAL_MANIFEST)).unwrap();
    write_draft_manifest(&directory, &manifest, true).unwrap();
    let graph = graph_with_drafts(&timeline(), &root, &state).unwrap();
    assert!(!graph.drafts[0].playable);
    assert!(materialize_draft(&timeline(), &root, &state, id).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn native_status_token_and_start_frame_must_match_before_ready() {
    let root = temporary_root("draft-status");
    write_tape(&root, "first.tape", &[1, 2, 3, 4]);
    write_tape(&root, "second.tape", &[5, 6, 7]);
    let state = root.join("state");
    let id = "draft-test-status";
    let mut manifest = install_ready_draft(&root, &state, id, &[8, 9]);
    let directory = drafts_root(&state).unwrap().join(id);
    manifest.status = DraftStatus::Recording;
    manifest.start_boundary_verified = false;
    manifest.tape_sha256 = None;
    manifest.tape_bytes = None;
    manifest.result_tape_sha256 = None;
    let status_path = directory.join(format!("{DRAFT_TAPE}.status.json"));
    let status = |token: &str, frame: u64| {
        serde_json::json!({
            "schema": "dusklight.input-recording/v2",
            "status": "success",
            "tape": fs::canonicalize(directory.join(DRAFT_TAPE)).unwrap(),
            "frame_count": 2,
            "frame_capacity": 1080000,
            "handoff_reached": true,
            "capacity_exhausted": false,
            "error": null,
            "process_success": true,
            "session_token": token,
            "start_milestone": "entered-f-sp104",
            "start_fingerprint": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "expected_start_fingerprint": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "start_boundary_kind": "tick",
            "start_boundary_index": null,
            "start_program_digest": null,
            "start_definition_digest": null,
            "start_tape_frame": frame
        })
    };

    fs::write(
        &status_path,
        serde_json::to_vec(&status("ffffffffffffffffffffffffffffffff", 6)).unwrap(),
    )
    .unwrap();
    let mut rejected = manifest.clone();
    finalize_recording(&directory, &mut rejected, Some(true));
    assert_eq!(rejected.status, DraftStatus::ProcessFailure);

    fs::write(
        &status_path,
        serde_json::to_vec(&status(&manifest.session_token, 5)).unwrap(),
    )
    .unwrap();
    let mut wrong_frame = manifest.clone();
    finalize_recording(&directory, &mut wrong_frame, Some(true));
    assert_eq!(wrong_frame.status, DraftStatus::ProcessFailure);

    let mut process_failed_status = status(&manifest.session_token, 6);
    process_failed_status["process_success"] = serde_json::json!(false);
    fs::write(
        &status_path,
        serde_json::to_vec(&process_failed_status).unwrap(),
    )
    .unwrap();
    let mut native_failed = manifest.clone();
    finalize_recording(&directory, &mut native_failed, None);
    assert_eq!(native_failed.status, DraftStatus::ProcessFailure);

    fs::write(
        &status_path,
        serde_json::to_vec(&status(&manifest.session_token, 6)).unwrap(),
    )
    .unwrap();
    let mut exit_disagreed = manifest.clone();
    finalize_recording(&directory, &mut exit_disagreed, Some(false));
    assert_eq!(exit_disagreed.status, DraftStatus::ProcessFailure);

    finalize_recording(&directory, &mut manifest, Some(true));
    assert_eq!(manifest.status, DraftStatus::Ready);
    assert!(manifest.start_boundary_verified);
    assert!(manifest.tape_sha256.as_deref().is_some_and(valid_sha256));
    assert!(
        manifest
            .result_tape_sha256
            .as_deref()
            .is_some_and(valid_sha256)
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cyclic_ready_drafts_are_structurally_nonplayable() {
    let root = temporary_root("draft-cycle");
    write_tape(&root, "first.tape", &[1, 2, 3, 4]);
    write_tape(&root, "second.tape", &[5, 6, 7]);
    let state = root.join("state");
    let mut left = install_ready_draft(&root, &state, "draft-cycle-left", &[8]);
    let mut right = install_ready_draft(&root, &state, "draft-cycle-right", &[9]);
    left.parent = DraftParent::Draft {
        id: right.id.clone(),
        parent_tape_sha256: right.result_tape_sha256.clone().unwrap(),
    };
    left.parent_tape_sha256 = right.result_tape_sha256.clone().unwrap();
    right.parent = DraftParent::Draft {
        id: left.id.clone(),
        parent_tape_sha256: left.result_tape_sha256.clone().unwrap(),
    };
    right.parent_tape_sha256 = left.result_tape_sha256.clone().unwrap();
    for manifest in [&left, &right] {
        let directory = drafts_root(&state).unwrap().join(&manifest.id);
        fs::remove_file(directory.join(DRAFT_FINAL_MANIFEST)).unwrap();
        write_draft_manifest(&directory, manifest, true).unwrap();
    }
    let graph = graph_with_drafts(&timeline(), &root, &state).unwrap();
    assert_eq!(graph.drafts.len(), 2);
    assert!(graph.drafts.iter().all(|draft| !draft.playable));
    assert!(
        graph
            .drafts
            .iter()
            .all(|draft| draft.error.as_deref().unwrap().contains("cycle"))
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn missing_and_nonready_draft_parents_block_children() {
    let root = temporary_root("draft-parent-state");
    write_tape(&root, "first.tape", &[1, 2, 3, 4]);
    write_tape(&root, "second.tape", &[5, 6, 7]);
    let state = root.join("state");
    let mut missing = install_ready_draft(&root, &state, "draft-missing-child", &[8]);
    missing.parent = DraftParent::Draft {
        id: "draft-does-not-exist".into(),
        parent_tape_sha256: "11".repeat(32),
    };
    missing.parent_tape_sha256 = "11".repeat(32);

    let mut parent = install_ready_draft(&root, &state, "draft-nonready-parent", &[9]);
    parent.status = DraftStatus::CapacityExhausted;
    let mut child = install_ready_draft(&root, &state, "draft-nonready-child", &[10]);
    child.parent = DraftParent::Draft {
        id: parent.id.clone(),
        parent_tape_sha256: parent.result_tape_sha256.clone().unwrap(),
    };
    child.parent_tape_sha256 = parent.result_tape_sha256.clone().unwrap();

    for manifest in [&missing, &parent, &child] {
        let directory = drafts_root(&state).unwrap().join(&manifest.id);
        fs::remove_file(directory.join(DRAFT_FINAL_MANIFEST)).unwrap();
        write_draft_manifest(&directory, manifest, true).unwrap();
    }
    let graph = graph_with_drafts(&timeline(), &root, &state).unwrap();
    let by_id = graph
        .drafts
        .iter()
        .map(|draft| (draft.id.as_str(), draft))
        .collect::<BTreeMap<_, _>>();
    assert!(!by_id["draft-missing-child"].playable);
    assert!(
        by_id["draft-missing-child"]
            .error
            .as_deref()
            .unwrap()
            .contains("missing")
    );
    assert!(!by_id["draft-nonready-parent"].playable);
    assert!(!by_id["draft-nonready-child"].playable);
    assert!(
        by_id["draft-nonready-child"]
            .error
            .as_deref()
            .unwrap()
            .contains("not ready")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn interrupted_final_manifest_write_never_exposes_false_ready() {
    let root = temporary_root("draft-interrupted-final");
    write_tape(&root, "first.tape", &[1, 2, 3, 4]);
    write_tape(&root, "second.tape", &[5, 6, 7]);
    let state = root.join("state");
    let id = "draft-interrupted-final";
    let mut manifest = install_ready_draft(&root, &state, id, &[8]);
    let directory = drafts_root(&state).unwrap().join(id);
    fs::remove_file(directory.join(DRAFT_FINAL_MANIFEST)).unwrap();
    manifest.status = DraftStatus::Recording;
    manifest.tape_sha256 = None;
    manifest.tape_bytes = None;
    manifest.result_tape_sha256 = None;
    write_draft_manifest(&directory, &manifest, false).unwrap();
    fs::write(directory.join(".draft-interrupted.tmp"), b"{\"status\":").unwrap();
    let graph = graph_with_drafts(&timeline(), &root, &state).unwrap();
    assert_eq!(graph.drafts.len(), 1);
    assert_eq!(graph.drafts[0].status, DraftStatus::Orphaned);
    assert!(!graph.drafts[0].playable);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn orphaned_descriptor_recovers_from_late_token_bound_status() {
    let root = temporary_root("draft-orphan-recovery");
    write_tape(&root, "first.tape", &[1, 2, 3, 4]);
    write_tape(&root, "second.tape", &[5, 6, 7]);
    let state = root.join("state");
    let id = "draft-orphan-recovery";
    let mut manifest = install_ready_draft(&root, &state, id, &[8]);
    let directory = drafts_root(&state).unwrap().join(id);
    fs::remove_file(directory.join(DRAFT_FINAL_MANIFEST)).unwrap();
    manifest.status = DraftStatus::Preparing;
    manifest.start_boundary_verified = false;
    manifest.tape_sha256 = None;
    manifest.tape_bytes = None;
    manifest.result_tape_sha256 = None;
    write_draft_manifest(&directory, &manifest, false).unwrap();

    let orphaned = graph_with_drafts(&timeline(), &root, &state).unwrap();
    assert_eq!(orphaned.drafts[0].status, DraftStatus::Orphaned);
    assert!(!directory.join(DRAFT_FINAL_MANIFEST).exists());

    write_success_status(&directory, &manifest, 1);
    let recovered = graph_with_drafts(&timeline(), &root, &state).unwrap();
    assert_eq!(recovered.drafts[0].status, DraftStatus::Ready);
    assert!(recovered.drafts[0].playable);
    assert!(directory.join(DRAFT_FINAL_MANIFEST).is_file());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn graph_poll_does_not_promote_while_recording_process_is_live() {
    let root = temporary_root("draft-live-race");
    write_tape(&root, "first.tape", &[1, 2, 3, 4]);
    write_tape(&root, "second.tape", &[5, 6, 7]);
    let state = root.join("state");
    let id = "draft-live-race";
    let mut manifest = install_ready_draft(&root, &state, id, &[8]);
    let directory = drafts_root(&state).unwrap().join(id);
    fs::remove_file(directory.join(DRAFT_FINAL_MANIFEST)).unwrap();
    manifest.status = DraftStatus::Preparing;
    manifest.start_boundary_verified = false;
    manifest.tape_sha256 = None;
    manifest.tape_bytes = None;
    manifest.result_tape_sha256 = None;
    write_draft_manifest(&directory, &manifest, false).unwrap();
    write_draft_launch(
        &directory,
        &DraftLaunch {
            schema: "dusklight.route-workbench.launch.v2".into(),
            id: id.into(),
            pid: std::process::id(),
            session_token: manifest.session_token.clone(),
        },
    )
    .unwrap();
    write_success_status(&directory, &manifest, 1);

    let live = graph_with_drafts(&timeline(), &root, &state).unwrap();
    assert_eq!(live.drafts[0].status, DraftStatus::Recording);
    assert!(!live.drafts[0].playable);
    assert!(!directory.join(DRAFT_FINAL_MANIFEST).exists());

    fs::remove_file(directory.join(DRAFT_LAUNCH)).unwrap();
    write_draft_launch(
        &directory,
        &DraftLaunch {
            schema: "dusklight.route-workbench.launch.v2".into(),
            id: id.into(),
            pid: u32::MAX,
            session_token: manifest.session_token.clone(),
        },
    )
    .unwrap();
    active_recordings().lock().unwrap().insert(id.into());
    let awaiting_monitor = graph_with_drafts(&timeline(), &root, &state).unwrap();
    assert_eq!(awaiting_monitor.drafts[0].status, DraftStatus::Recording);
    assert!(!directory.join(DRAFT_FINAL_MANIFEST).exists());
    active_recordings().lock().unwrap().remove(id);
    let exited = graph_with_drafts(&timeline(), &root, &state).unwrap();
    assert_eq!(exited.drafts[0].status, DraftStatus::Ready);
    assert!(exited.drafts[0].playable);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn segment_rename_edits_only_git_owned_display_metadata() {
    let root = temporary_root("segment-rename");
    let path = root.join("route.timeline");
    fs::write(&path, milestone_timeline_source()).unwrap();

    let first = rename_segment(
        &path,
        &BrowserSegmentRenameRequest {
            id: "boot_link.one".into(),
            name: "  Boot to Link control  ".into(),
            expected_name: None,
        },
    )
    .unwrap();
    assert_eq!(first.name, "Boot to Link control");
    let first_source = fs::read_to_string(&path).unwrap();
    assert_eq!(
        first_source
            .lines()
            .filter(|line| line.starts_with("label boot_link.one "))
            .count(),
        1
    );
    let first_timeline = Timeline::parse(&first_source).unwrap();
    let segment = &first_timeline.segments["boot_link.one"];
    assert_eq!(segment.name.as_deref(), Some("Boot to Link control"));
    assert_eq!(segment.parent, None);
    assert_eq!(segment.end_fingerprint, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    assert_eq!(first_timeline.goals["control"].segment, "boot_link.one");

    let stale = rename_segment(
        &path,
        &BrowserSegmentRenameRequest {
            id: "boot_link.one".into(),
            name: "stale".into(),
            expected_name: None,
        },
    );
    assert!(matches!(stale, Err(SegmentRenameError::Conflict(_))));

    rename_segment(
        &path,
        &BrowserSegmentRenameRequest {
            id: "boot_link.one".into(),
            name: "Fast boot".into(),
            expected_name: Some("Boot to Link control".into()),
        },
    )
    .unwrap();
    let second_source = fs::read_to_string(&path).unwrap();
    assert_eq!(second_source.matches("label boot_link.one ").count(), 1);
    assert_eq!(
        Timeline::parse(&second_source).unwrap().segments["boot_link.one"]
            .name
            .as_deref(),
        Some("Fast boot")
    );
    assert!(fs::read_dir(&root).unwrap().all(|entry| {
        let name = entry.unwrap().file_name().to_string_lossy().into_owned();
        !name.ends_with(".tmp") && !name.ends_with(".rollback")
    }));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn segment_rename_http_rejects_identity_and_path_smuggling() {
    let root = temporary_root("segment-rename-http");
    let timeline_path = root.join("route.timeline");
    fs::write(&timeline_path, milestone_timeline_source()).unwrap();
    let config = WorkbenchConfig {
        timeline_path: timeline_path.clone(),
        repository_root: root.clone(),
        working_directory: root.clone(),
        game: root.join("unused-game"),
        dvd: root.join("unused-dvd"),
        state_root: root.join("state"),
    };
    let smuggled = serde_json::json!({
        "id": "boot_link.one",
        "name": "renamed",
        "expected_name": null,
        "path": "../outside.timeline",
        "new_id": "different-segment"
    });
    let rejected = call_http(
        &config,
        "POST",
        "/api/segments/rename",
        &serde_json::to_vec(&smuggled).unwrap(),
    );
    assert_eq!(rejected.status, 400);

    let request = BrowserSegmentRenameRequest {
        id: "boot_link.one".into(),
        name: "Named through HTTP".into(),
        expected_name: None,
    };
    let response = call_http(
        &config,
        "POST",
        "/api/segments/rename",
        &serde_json::to_vec(&request).unwrap(),
    );
    assert_eq!(response.status, 200);
    let body: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(body["id"], "boot_link.one");
    assert_eq!(body["name"], "Named through HTTP");
    assert!(body.get("path").is_none());
    assert!(body.get("new_id").is_none());
    let stale = call_http(
        &config,
        "POST",
        "/api/segments/rename",
        &serde_json::to_vec(&request).unwrap(),
    );
    assert_eq!(stale.status, 409);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn segment_delete_rewrites_only_the_selected_structural_subtree() {
    let source = milestone_timeline_source();
    let deletion = delete_segment_subtree_in_timeline_source(&source, "link_exit.one").unwrap();
    assert_eq!(deletion.segments, BTreeSet::from(["link_exit.one".into()]));
    assert_eq!(deletion.goals, BTreeSet::from(["exit".into()]));
    assert_eq!(deletion.proofs, 1);
    assert!(deletion.lineages.is_empty());
    assert!(!deletion.replacement.contains("segment link_exit.one "));
    assert!(!deletion.replacement.contains("goal exit "));
    assert!(!deletion.replacement.contains("proof link_exit.one "));
    assert!(
        !deletion
            .replacement
            .contains("continue main with link_exit.one ")
    );
    assert!(deletion.replacement.contains("segment boot_link.one "));
    assert!(deletion.replacement.contains("goal control "));
    assert!(deletion.replacement.contains("proof boot_link.one "));
    assert!(deletion.replacement.contains("continuation main "));
    let parsed = Timeline::parse(&deletion.replacement).unwrap();
    assert_eq!(
        parsed.segments.keys().collect::<Vec<_>>(),
        vec!["boot_link.one"]
    );

    let root_deletion =
        delete_segment_subtree_in_timeline_source(&source, "boot_link.one").unwrap();
    assert_eq!(root_deletion.segments.len(), 2);
    assert_eq!(root_deletion.goals.len(), 2);
    assert_eq!(root_deletion.proofs, 2);
    assert_eq!(root_deletion.lineages, BTreeSet::from(["main".into()]));
    let empty = Timeline::parse(&root_deletion.replacement).unwrap();
    assert!(empty.segments.is_empty());
    assert!(empty.goals.is_empty());
    assert!(empty.proofs.is_empty());
    assert!(empty.continuations.is_empty());
}

#[test]
fn segment_delete_moves_attached_draft_closure_and_rejects_stale_or_active_state() {
    let root = temporary_root("segment-delete");
    write_tape(&root, "first.tape", &[1, 2, 3, 4]);
    write_tape(&root, "second.tape", &[5, 6, 7]);
    let timeline_path = root.join("route.timeline");
    fs::write(&timeline_path, milestone_timeline_source()).unwrap();
    let state = root.join("state");
    let parent = install_ready_draft(&root, &state, "draft-segment-delete-parent", &[8]);
    let mut child = install_ready_draft(&root, &state, "draft-segment-delete-child", &[9]);
    child.parent = DraftParent::Draft {
        id: parent.id.clone(),
        parent_tape_sha256: parent.result_tape_sha256.clone().unwrap(),
    };
    child.parent_tape_sha256 = parent.result_tape_sha256.clone().unwrap();
    let child_directory = drafts_root(&state).unwrap().join(&child.id);
    fs::remove_file(child_directory.join(DRAFT_FINAL_MANIFEST)).unwrap();
    write_draft_manifest(&child_directory, &child, true).unwrap();

    let installed = scan_draft_manifests(&state).unwrap();
    assert_eq!(installed.len(), 2);
    assert!(matches!(
        &installed[&parent.id].parent,
        DraftParent::Segment { id, .. } if id == "link_exit.one"
    ));

    active_recordings().lock().unwrap().insert(child.id.clone());
    let active = preview_segment_deletion(&timeline_path, &state, "link_exit.one");
    active_recordings().lock().unwrap().remove(&child.id);
    assert!(active.unwrap_err().to_string().contains("active"));

    let stale_preview = preview_segment_deletion(&timeline_path, &state, "link_exit.one").unwrap();
    fs::write(
        &timeline_path,
        format!("{}\n# changed after preview\n", milestone_timeline_source()),
    )
    .unwrap();
    let stale = apply_segment_deletion(
        &timeline_path,
        &state,
        &BrowserSegmentDeleteApplyRequest {
            id: "link_exit.one".into(),
            confirmation_token: stale_preview.confirmation_token,
        },
    );
    assert!(matches!(stale, Err(SegmentDeleteError::Conflict(_))));
    assert!(drafts_root(&state).unwrap().join(&parent.id).is_dir());
    assert!(drafts_root(&state).unwrap().join(&child.id).is_dir());

    fs::write(&timeline_path, milestone_timeline_source()).unwrap();
    let preview = preview_segment_deletion(&timeline_path, &state, "link_exit.one").unwrap();
    assert_eq!(preview.segments.len(), 1);
    assert_eq!(preview.goals, vec!["exit"]);
    assert_eq!(preview.proofs, 1);
    assert_eq!(preview.drafts.len(), 2);
    let result = apply_segment_deletion(
        &timeline_path,
        &state,
        &BrowserSegmentDeleteApplyRequest {
            id: "link_exit.one".into(),
            confirmation_token: preview.confirmation_token,
        },
    )
    .unwrap();
    assert_eq!(result.segments, vec!["link_exit.one"]);
    assert_eq!(result.drafts.len(), 2);
    assert!(result.trash_transaction.unwrap().join(&parent.id).is_dir());
    assert!(root.join("second.tape").is_file());
    let timeline = Timeline::parse(&fs::read_to_string(&timeline_path).unwrap()).unwrap();
    assert_eq!(
        timeline.segments.keys().collect::<Vec<_>>(),
        vec!["boot_link.one"]
    );
    assert!(!drafts_root(&state).unwrap().join(&parent.id).exists());
    assert!(!drafts_root(&state).unwrap().join(&child.id).exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn sibling_delete_keeps_selected_subtree_and_removes_every_other_sibling_subtree() {
    let root = temporary_root("sibling-delete");
    let timeline_path = root.join("route.timeline");
    fs::write(&timeline_path, sibling_timeline_source()).unwrap();
    for artifact in [
        "root.tape",
        "left.tape",
        "left-child.tape",
        "keep.tape",
        "keep-child.tape",
        "right.tape",
    ] {
        fs::write(root.join(artifact), artifact.as_bytes()).unwrap();
    }
    let state = root.join("state");
    write_tape(&root, "first.tape", &[1, 2, 3]);
    write_tape(&root, "second.tape", &[4, 5]);
    let mut direct = install_ready_draft(&root, &state, "draft-direct-sibling", &[6]);
    direct.parent = DraftParent::Segment {
        id: "root".into(),
        terminal_milestone: "unused".into(),
        boundary_fingerprint: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
    };
    let direct_directory = drafts_root(&state).unwrap().join(&direct.id);
    fs::remove_file(direct_directory.join(DRAFT_FINAL_MANIFEST)).unwrap();
    write_draft_manifest(&direct_directory, &direct, true).unwrap();
    let mut child = install_ready_draft(&root, &state, "draft-direct-child", &[7]);
    child.parent = DraftParent::Draft {
        id: direct.id.clone(),
        parent_tape_sha256: direct.result_tape_sha256.clone().unwrap(),
    };
    child.parent_tape_sha256 = direct.result_tape_sha256.clone().unwrap();
    let child_directory = drafts_root(&state).unwrap().join(&child.id);
    fs::remove_file(child_directory.join(DRAFT_FINAL_MANIFEST)).unwrap();
    write_draft_manifest(&child_directory, &child, true).unwrap();

    let preview = preview_sibling_deletion(&timeline_path, &root, &state, "keep").unwrap();
    assert_eq!(preview.keep_id, "keep");
    assert_eq!(
        preview
            .sibling_roots
            .iter()
            .map(|segment| segment.id.as_str())
            .collect::<Vec<_>>(),
        vec!["left", "right"]
    );
    assert_eq!(
        preview
            .segments
            .iter()
            .map(|segment| segment.id.as_str())
            .collect::<Vec<_>>(),
        vec!["left", "left_child", "right"]
    );
    assert_eq!(
        preview
            .draft_roots
            .iter()
            .map(|draft| draft.id.as_str())
            .collect::<Vec<_>>(),
        vec!["draft-direct-sibling"]
    );
    assert_eq!(preview.drafts.len(), 2);
    let result = apply_sibling_deletion(
        &timeline_path,
        &root,
        &state,
        &BrowserSiblingDeleteApplyRequest {
            keep_id: "keep".into(),
            confirmation_token: preview.confirmation_token,
        },
    )
    .unwrap();
    assert_eq!(result.sibling_roots, vec!["left", "right"]);
    assert_eq!(result.segments, vec!["left", "left_child", "right"]);
    assert_eq!(result.draft_roots, vec!["draft-direct-sibling"]);
    assert_eq!(result.drafts.len(), 2);
    assert!(!direct_directory.exists());
    assert!(!child_directory.exists());

    let timeline = Timeline::parse(&fs::read_to_string(&timeline_path).unwrap()).unwrap();
    assert_eq!(
        timeline
            .segments
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec!["keep", "keep_child", "root"]
    );
    for artifact in [
        "root.tape",
        "left.tape",
        "left-child.tape",
        "keep.tape",
        "keep-child.tape",
        "right.tape",
    ] {
        assert!(
            root.join(artifact).is_file(),
            "artifact {artifact} was removed"
        );
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn sibling_delete_reanchors_a_deleted_reference_goal_to_its_proved_survivor() {
    let root = temporary_root("sibling-delete-goal-reanchor");
    let timeline_path = root.join("route.timeline");
    fs::write(&timeline_path, sibling_timeline_with_shared_goal_source()).unwrap();
    let state = root.join("state");

    let preview = preview_sibling_deletion(&timeline_path, &root, &state, "keep").unwrap();
    assert!(preview.goals.is_empty());
    assert_eq!(preview.proofs, 1);
    assert!(preview.lineages.is_empty());
    assert_eq!(
        preview
            .sibling_roots
            .iter()
            .map(|segment| segment.id.as_str())
            .collect::<Vec<_>>(),
        vec!["incumbent", "unrelated_profile"]
    );

    apply_sibling_deletion(
        &timeline_path,
        &root,
        &state,
        &BrowserSiblingDeleteApplyRequest {
            keep_id: "keep".into(),
            confirmation_token: preview.confirmation_token,
        },
    )
    .unwrap();

    let replacement = fs::read_to_string(&timeline_path).unwrap();
    assert!(replacement.contains("goal destination on keep predicate destination"));
    assert!(!replacement.contains("proof incumbent satisfies destination"));
    assert!(replacement.contains("proof keep satisfies destination"));
    assert!(
        replacement.contains("continue main with keep after root@aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    );
    assert!(!replacement.contains("continue main with incumbent"));
    let timeline = Timeline::parse(&replacement).unwrap();
    assert_eq!(timeline.goals["destination"].segment, "keep");
    assert_eq!(timeline.proofs.len(), 1);
    assert_eq!(timeline.proofs[0].segment, "keep");
    assert_eq!(timeline.proofs[0].first_hit_tick, Some(129));
    assert_eq!(timeline.continuations["main"].steps[1].segment, "keep");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn sibling_delete_rejects_roots_lonely_segments_stale_tokens_and_smuggled_fields() {
    let root = temporary_root("sibling-delete-guards");
    let timeline_path = root.join("route.timeline");
    fs::write(&timeline_path, sibling_timeline_source()).unwrap();
    let state = root.join("state");
    assert!(
        preview_sibling_deletion(&timeline_path, &root, &state, "root")
            .unwrap_err()
            .to_string()
            .contains("root segment")
    );
    assert!(
        preview_sibling_deletion(&timeline_path, &root, &state, "keep_child")
            .unwrap_err()
            .to_string()
            .contains("no displayed siblings")
    );
    assert!(preview_sibling_deletion(&timeline_path, &root, &state, "../keep").is_err());

    write_tape(&root, "first.tape", &[1, 2, 3]);
    write_tape(&root, "second.tape", &[4, 5]);
    let mut active_draft = install_ready_draft(&root, &state, "draft-active-sibling", &[6]);
    active_draft.parent = DraftParent::Segment {
        id: "left".into(),
        terminal_milestone: "unused".into(),
        boundary_fingerprint: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
    };
    let active_directory = drafts_root(&state).unwrap().join(&active_draft.id);
    fs::remove_file(active_directory.join(DRAFT_FINAL_MANIFEST)).unwrap();
    write_draft_manifest(&active_directory, &active_draft, true).unwrap();
    active_recordings()
        .lock()
        .unwrap()
        .insert(active_draft.id.clone());
    let active = preview_sibling_deletion(&timeline_path, &root, &state, "keep");
    active_recordings().lock().unwrap().remove(&active_draft.id);
    assert!(active.unwrap_err().to_string().contains("active"));
    fs::remove_dir_all(active_directory).unwrap();

    let preview = preview_sibling_deletion(&timeline_path, &root, &state, "keep").unwrap();
    fs::write(
        &timeline_path,
        format!("{}\n# topology revision\n", sibling_timeline_source()),
    )
    .unwrap();
    let stale = apply_sibling_deletion(
        &timeline_path,
        &root,
        &state,
        &BrowserSiblingDeleteApplyRequest {
            keep_id: "keep".into(),
            confirmation_token: preview.confirmation_token,
        },
    );
    assert!(matches!(stale, Err(SegmentDeleteError::Conflict(_))));

    let config = WorkbenchConfig {
        timeline_path,
        repository_root: root.clone(),
        working_directory: root.clone(),
        game: root.join("unused-game"),
        dvd: root.join("unused-dvd"),
        state_root: state,
    };
    let response = call_http(
        &config,
        "POST",
        "/api/segments/delete-siblings/preview",
        &serde_json::to_vec(&serde_json::json!({
            "keep_id": "keep",
            "path": "../outside.timeline"
        }))
        .unwrap(),
    );
    assert_eq!(response.status, 400);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn draft_rename_changes_only_final_manifest_label() {
    let root = temporary_root("draft-rename");
    write_tape(&root, "first.tape", &[1, 2, 3, 4]);
    write_tape(&root, "second.tape", &[5, 6, 7]);
    let state = root.join("state");
    let parent = install_ready_draft(&root, &state, "draft-rename-parent", &[8]);
    let mut child = install_ready_draft(&root, &state, "draft-rename-child", &[9]);
    child.parent = DraftParent::Draft {
        id: parent.id.clone(),
        parent_tape_sha256: parent.result_tape_sha256.clone().unwrap(),
    };
    child.parent_tape_sha256 = parent.result_tape_sha256.clone().unwrap();
    let child_directory = drafts_root(&state).unwrap().join(&child.id);
    fs::remove_file(child_directory.join(DRAFT_FINAL_MANIFEST)).unwrap();
    write_draft_manifest(&child_directory, &child, true).unwrap();

    let draft_root = drafts_root(&state).unwrap();
    let directory = draft_root.join(&parent.id);
    let manifest_path = directory.join(DRAFT_FINAL_MANIFEST);
    let tape_before = fs::read(directory.join(DRAFT_TAPE)).unwrap();
    let manifest_before: DraftManifest =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    let manifests = scan_draft_manifests(&state).unwrap();
    let revision = draft_graph_revision(&manifests).unwrap();
    let graph = graph_with_drafts(&timeline(), &root, &state).unwrap();
    assert_eq!(
        graph.draft_graph_revision.as_deref(),
        Some(revision.as_str())
    );
    let result = rename_draft_label(
        &state,
        &BrowserDraftRenameRequest {
            id: parent.id.clone(),
            label: "  Useful route  ".into(),
            expected_graph_revision: revision.clone(),
        },
    )
    .unwrap();
    assert_eq!(result.label, "Useful route");
    assert_ne!(result.graph_revision, revision);
    assert!(directory.is_dir());
    assert_eq!(fs::read(directory.join(DRAFT_TAPE)).unwrap(), tape_before);
    let manifest_after: DraftManifest =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    let mut expected = manifest_before;
    expected.label = "Useful route".into();
    assert_eq!(
        serde_json::to_value(manifest_after).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
    let rescanned = scan_draft_manifests(&state).unwrap();
    let DraftParent::Draft { id, .. } = &rescanned[&child.id].parent else {
        panic!("child lost its draft parent");
    };
    assert_eq!(id, &parent.id);
    assert!(fs::read_dir(&directory).unwrap().all(|entry| {
        let name = entry.unwrap().file_name().to_string_lossy().into_owned();
        !name.ends_with(".tmp") && !name.ends_with(".rollback")
    }));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn draft_rename_rejects_active_stale_and_invalid_requests_without_writing() {
    let root = temporary_root("draft-rename-conflict");
    write_tape(&root, "first.tape", &[1, 2, 3, 4]);
    write_tape(&root, "second.tape", &[5, 6, 7]);
    let state = root.join("state");
    let target = install_ready_draft(&root, &state, "draft-rename-target", &[8]);
    let mut sibling = install_ready_draft(&root, &state, "draft-rename-sibling", &[9]);
    let directory = drafts_root(&state).unwrap().join(&target.id);
    let manifest_path = directory.join(DRAFT_FINAL_MANIFEST);
    let original = fs::read(&manifest_path).unwrap();
    let revision = draft_graph_revision(&scan_draft_manifests(&state).unwrap()).unwrap();

    active_recordings()
        .lock()
        .unwrap()
        .insert(target.id.clone());
    let active = rename_draft_label(
        &state,
        &BrowserDraftRenameRequest {
            id: target.id.clone(),
            label: "blocked".into(),
            expected_graph_revision: revision.clone(),
        },
    );
    active_recordings().lock().unwrap().remove(&target.id);
    assert!(matches!(active, Err(DraftRenameError::Conflict(_))));
    assert_eq!(fs::read(&manifest_path).unwrap(), original);

    sibling.label = "concurrent sibling edit".into();
    let sibling_directory = drafts_root(&state).unwrap().join(&sibling.id);
    fs::remove_file(sibling_directory.join(DRAFT_FINAL_MANIFEST)).unwrap();
    write_draft_manifest(&sibling_directory, &sibling, true).unwrap();
    let stale = rename_draft_label(
        &state,
        &BrowserDraftRenameRequest {
            id: target.id.clone(),
            label: "stale".into(),
            expected_graph_revision: revision,
        },
    );
    assert!(matches!(stale, Err(DraftRenameError::Conflict(_))));
    assert_eq!(fs::read(&manifest_path).unwrap(), original);

    let current = draft_graph_revision(&scan_draft_manifests(&state).unwrap()).unwrap();
    for label in [
        String::new(),
        "   ".into(),
        "bad\nlabel".into(),
        "x".repeat(161),
    ] {
        let invalid = rename_draft_label(
            &state,
            &BrowserDraftRenameRequest {
                id: target.id.clone(),
                label,
                expected_graph_revision: current.clone(),
            },
        );
        assert!(matches!(invalid, Err(DraftRenameError::Invalid(_))));
        assert_eq!(fs::read(&manifest_path).unwrap(), original);
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn draft_rename_http_api_rejects_paths_and_returns_stale_conflict() {
    let root = temporary_root("draft-rename-http");
    write_tape(&root, "first.tape", &[1, 2, 3, 4]);
    write_tape(&root, "second.tape", &[5, 6, 7]);
    let state = root.join("state");
    let draft = install_ready_draft(&root, &state, "draft-rename-http", &[8]);
    let revision = draft_graph_revision(&scan_draft_manifests(&state).unwrap()).unwrap();
    let config = WorkbenchConfig {
        timeline_path: root.join("unused.timeline"),
        repository_root: root.clone(),
        working_directory: root.clone(),
        game: root.join("unused-game"),
        dvd: root.join("unused-dvd"),
        state_root: state.clone(),
    };
    let smuggled = serde_json::json!({
        "id": draft.id.clone(),
        "label": "renamed",
        "expected_graph_revision": revision.clone(),
        "path": "../outside",
        "new_id": "replacement-id"
    });
    let rejected = call_http(
        &config,
        "POST",
        "/api/drafts/rename",
        &serde_json::to_vec(&smuggled).unwrap(),
    );
    assert_eq!(rejected.status, 400);

    let request = BrowserDraftRenameRequest {
        id: draft.id.clone(),
        label: "renamed through HTTP".into(),
        expected_graph_revision: revision,
    };
    let response = call_http(
        &config,
        "POST",
        "/api/drafts/rename",
        &serde_json::to_vec(&request).unwrap(),
    );
    assert_eq!(response.status, 200);
    let body: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(body["id"], draft.id);
    assert_eq!(body["label"], "renamed through HTTP");
    assert!(body.get("path").is_none());
    let stale = call_http(
        &config,
        "POST",
        "/api/drafts/rename",
        &serde_json::to_vec(&request).unwrap(),
    );
    assert_eq!(stale.status, 409);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn draft_rename_rejects_manifest_symlink_escape() {
    use std::os::unix::fs::symlink;

    let root = temporary_root("draft-rename-symlink");
    write_tape(&root, "first.tape", &[1, 2, 3, 4]);
    write_tape(&root, "second.tape", &[5, 6, 7]);
    let state = root.join("state");
    let draft = install_ready_draft(&root, &state, "draft-rename-symlink", &[8]);
    let revision = draft_graph_revision(&scan_draft_manifests(&state).unwrap()).unwrap();
    let directory = drafts_root(&state).unwrap().join(&draft.id);
    let final_path = directory.join(DRAFT_FINAL_MANIFEST);
    let outside = root.join("outside.json");
    fs::write(&outside, fs::read(&final_path).unwrap()).unwrap();
    fs::remove_file(&final_path).unwrap();
    symlink(&outside, &final_path).unwrap();
    let result = rename_draft_label(
        &state,
        &BrowserDraftRenameRequest {
            id: draft.id,
            label: "escaped".into(),
            expected_graph_revision: revision,
        },
    );
    assert!(result.is_err());
    let outside_manifest: DraftManifest =
        serde_json::from_slice(&fs::read(&outside).unwrap()).unwrap();
    assert_eq!(outside_manifest.label, "Test branch");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn draft_delete_moves_only_selected_subtree_to_recoverable_trash() {
    let root = temporary_root("draft-delete-subtree");
    write_tape(&root, "first.tape", &[1, 2, 3, 4]);
    write_tape(&root, "second.tape", &[5, 6, 7]);
    let state = root.join("state");
    let base = install_ready_draft(&root, &state, "draft-delete-base", &[8]);
    let mut selected = install_ready_draft(&root, &state, "draft-delete-selected", &[9]);
    let mut sibling = install_ready_draft(&root, &state, "draft-delete-sibling", &[10]);
    let mut descendant = install_ready_draft(&root, &state, "draft-delete-descendant", &[11]);
    for child in [&mut selected, &mut sibling] {
        child.parent = DraftParent::Draft {
            id: base.id.clone(),
            parent_tape_sha256: base.result_tape_sha256.clone().unwrap(),
        };
        child.parent_tape_sha256 = base.result_tape_sha256.clone().unwrap();
    }
    descendant.parent = DraftParent::Draft {
        id: selected.id.clone(),
        parent_tape_sha256: selected.result_tape_sha256.clone().unwrap(),
    };
    descendant.parent_tape_sha256 = selected.result_tape_sha256.clone().unwrap();
    for manifest in [&selected, &sibling, &descendant] {
        let directory = drafts_root(&state).unwrap().join(&manifest.id);
        fs::remove_file(directory.join(DRAFT_FINAL_MANIFEST)).unwrap();
        write_draft_manifest(&directory, manifest, true).unwrap();
    }

    let preview = preview_draft_deletion(&state, &selected.id).unwrap();
    assert_eq!(
        preview
            .drafts
            .iter()
            .map(|draft| draft.id.as_str())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([selected.id.as_str(), descendant.id.as_str()])
    );
    let result = apply_draft_deletion(
        &state,
        &BrowserDraftDeleteApplyRequest {
            id: selected.id.clone(),
            confirmation_token: preview.confirmation_token,
        },
    )
    .unwrap();
    let draft_root = drafts_root(&state).unwrap();
    assert!(draft_root.join(&base.id).is_dir());
    assert!(draft_root.join(&sibling.id).is_dir());
    assert!(!draft_root.join(&selected.id).exists());
    assert!(!draft_root.join(&descendant.id).exists());
    assert!(result.trash_transaction.join(&selected.id).is_dir());
    assert!(result.trash_transaction.join(&descendant.id).is_dir());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn draft_delete_rejects_active_recordings_and_stale_graph_tokens() {
    let root = temporary_root("draft-delete-stale");
    write_tape(&root, "first.tape", &[1, 2, 3, 4]);
    write_tape(&root, "second.tape", &[5, 6, 7]);
    let state = root.join("state");
    let id = "draft-delete-stale";
    let mut manifest = install_ready_draft(&root, &state, id, &[8]);

    active_recordings().lock().unwrap().insert(id.into());
    let active_result = preview_draft_deletion(&state, id);
    active_recordings().lock().unwrap().remove(id);
    assert!(active_result.unwrap_err().to_string().contains("active"));

    let preview = preview_draft_deletion(&state, id).unwrap();
    manifest.label = "changed after preview".into();
    let directory = drafts_root(&state).unwrap().join(id);
    fs::remove_file(directory.join(DRAFT_FINAL_MANIFEST)).unwrap();
    write_draft_manifest(&directory, &manifest, true).unwrap();
    let apply = apply_draft_deletion(
        &state,
        &BrowserDraftDeleteApplyRequest {
            id: id.into(),
            confirmation_token: preview.confirmation_token,
        },
    );
    assert!(
        apply
            .unwrap_err()
            .to_string()
            .contains("changed after preview")
    );
    assert!(directory.is_dir());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn draft_delete_rejects_descendant_added_after_preview() {
    let root = temporary_root("draft-delete-new-descendant");
    write_tape(&root, "first.tape", &[1, 2, 3, 4]);
    write_tape(&root, "second.tape", &[5, 6, 7]);
    let state = root.join("state");
    let parent = install_ready_draft(&root, &state, "draft-delete-parent", &[8]);
    let preview = preview_draft_deletion(&state, &parent.id).unwrap();
    let mut child = install_ready_draft(&root, &state, "draft-delete-late-child", &[9]);
    child.parent = DraftParent::Draft {
        id: parent.id.clone(),
        parent_tape_sha256: parent.result_tape_sha256.clone().unwrap(),
    };
    child.parent_tape_sha256 = parent.result_tape_sha256.clone().unwrap();
    let child_directory = drafts_root(&state).unwrap().join(&child.id);
    fs::remove_file(child_directory.join(DRAFT_FINAL_MANIFEST)).unwrap();
    write_draft_manifest(&child_directory, &child, true).unwrap();

    let result = apply_draft_deletion(
        &state,
        &BrowserDraftDeleteApplyRequest {
            id: parent.id.clone(),
            confirmation_token: preview.confirmation_token,
        },
    );
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("changed after preview")
    );
    let draft_root = drafts_root(&state).unwrap();
    assert!(draft_root.join(parent.id).is_dir());
    assert!(draft_root.join(child.id).is_dir());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn draft_delete_descendant_closure_is_cycle_safe() {
    let root = temporary_root("draft-delete-cycle");
    write_tape(&root, "first.tape", &[1, 2, 3, 4]);
    write_tape(&root, "second.tape", &[5, 6, 7]);
    let state = root.join("state");
    let mut left = install_ready_draft(&root, &state, "draft-delete-cycle-left", &[8]);
    let mut right = install_ready_draft(&root, &state, "draft-delete-cycle-right", &[9]);
    left.parent = DraftParent::Draft {
        id: right.id.clone(),
        parent_tape_sha256: right.result_tape_sha256.clone().unwrap(),
    };
    right.parent = DraftParent::Draft {
        id: left.id.clone(),
        parent_tape_sha256: left.result_tape_sha256.clone().unwrap(),
    };
    for manifest in [&left, &right] {
        let directory = drafts_root(&state).unwrap().join(&manifest.id);
        fs::remove_file(directory.join(DRAFT_FINAL_MANIFEST)).unwrap();
        write_draft_manifest(&directory, manifest, true).unwrap();
    }
    let preview = preview_draft_deletion(&state, &left.id).unwrap();
    assert_eq!(preview.drafts.len(), 2);
    assert!(preview.drafts.iter().any(|draft| draft.id == left.id));
    assert!(preview.drafts.iter().any(|draft| draft.id == right.id));
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn draft_delete_refuses_directory_symlink_escape_after_preview() {
    use std::os::unix::fs::symlink;

    let root = temporary_root("draft-delete-symlink");
    write_tape(&root, "first.tape", &[1, 2, 3, 4]);
    write_tape(&root, "second.tape", &[5, 6, 7]);
    let state = root.join("state");
    let id = "draft-delete-symlink";
    install_ready_draft(&root, &state, id, &[8]);
    let preview = preview_draft_deletion(&state, id).unwrap();
    let directory = drafts_root(&state).unwrap().join(id);
    let escaped = root.join("escaped-draft");
    fs::rename(&directory, &escaped).unwrap();
    symlink(&escaped, &directory).unwrap();
    let result = apply_draft_deletion(
        &state,
        &BrowserDraftDeleteApplyRequest {
            id: id.into(),
            confirmation_token: preview.confirmation_token,
        },
    );
    assert!(result.is_err());
    assert!(escaped.is_dir());
    fs::remove_dir_all(root).unwrap();
}
