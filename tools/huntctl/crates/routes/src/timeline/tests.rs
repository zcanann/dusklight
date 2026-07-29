use super::*;

const ROUTE: &str = r#"
timeline intro
predicate_program route.milestones
origin boot predicate process_boot
segment boot_safe root profile boot_to_fsp103 uses baseline boot_to_fsp103 starts clean-rng1 produces control-rng1
label boot_safe "Conservative boot"
segment boot_fast root profile boot_to_fsp103 uses baseline boot_to_fsp103 starts clean-rng1 produces control-rng1
segment boot_other_rng root profile boot_to_fsp103 uses baseline boot_to_fsp103 starts clean-rng2 produces control-rng2
segment exit_safe after boot_safe profile fsp103_to_fsp104 uses baseline fsp103_to_fsp104 starts control-rng1 produces map-rng1
segment exit_rolls after boot_safe profile fsp103_to_fsp104 uses baseline fsp103_to_fsp104 starts control-rng1 produces map-rng1
segment exit_repaired after boot_fast profile fsp103_to_fsp104 uses baseline fsp103_to_fsp104 starts control-rng1 produces map-rng1
goal link_control on boot_safe predicate link_control
goal next_map on exit_safe predicate next_map
proof boot_safe satisfies link_control program 2222222222222222222222222222222222222222222222222222222222222222 predicate 1111111111111111111111111111111111111111111111111111111111111111 ticks 700
proof boot_fast satisfies link_control program 2222222222222222222222222222222222222222222222222222222222222222 predicate 1111111111111111111111111111111111111111111111111111111111111111 ticks 650
continuation main starts root@clean-rng1
continue main with boot_safe after root@clean-rng1
continue main with exit_safe after boot_safe@control-rng1
branch rolls from main after boot_safe
continue rolls with exit_rolls after boot_safe@control-rng1
"#;

#[test]
fn parses_segment_tree_continuations_branches_and_goal_frontiers() {
    let timeline = Timeline::parse(ROUTE).unwrap();
    let inspection = timeline.inspect().unwrap();
    assert_eq!(inspection.timeline.name, "intro");
    assert_eq!(inspection.lineages.len(), 2);
    assert_eq!(timeline.segments["boot_fast"].parent, None);
    assert_eq!(
        timeline.segments["boot_safe"].name.as_deref(),
        Some("Conservative boot")
    );
    assert_eq!(
        timeline.segments["exit_safe"].parent.as_deref(),
        Some("boot_safe")
    );
    let rolls = inspection
        .lineages
        .iter()
        .find(|lineage| lineage.name == "rolls")
        .unwrap();
    assert_eq!(rolls.steps[0].segment, "boot_safe");
    assert_eq!(rolls.steps[1].segment, "exit_rolls");
    let frontier = inspection
        .frontiers
        .iter()
        .find(|frontier| {
            frontier.reference_segment == "boot_safe"
                && frontier.goal == "link_control"
                && frontier.segments.len() == 2
        })
        .unwrap();
    assert_eq!(
        frontier
            .segments
            .iter()
            .find(|member| member.segment == "boot_fast")
            .unwrap()
            .relation_to_reference,
        DominanceRelation::Faster
    );
}

#[test]
fn parses_declared_process_boot_card_fixture() {
    let source = ROUTE.replace(
        "origin boot predicate process_boot",
        "origin boot predicate process_boot source process_boot.milestones card_fixture fixtures/process_boot.card",
    );
    let timeline = Timeline::parse(&source).unwrap();
    let origin = timeline.origin.as_ref().unwrap();
    assert_eq!(
        origin.predicate_source.as_deref(),
        Some(Path::new("process_boot.milestones"))
    );
    assert_eq!(
        origin.card_fixture.as_deref(),
        Some(Path::new("fixtures/process_boot.card"))
    );
}

#[test]
fn goal_proof_is_segment_metadata_and_may_be_satisfied_by_a_sibling() {
    let program_digest = "2".repeat(64);
    let digest = "1".repeat(64);
    let timeline = Timeline::parse(ROUTE).unwrap();
    let proof = &timeline.proofs[0];
    assert_eq!(proof.segment, "boot_safe");
    assert_eq!(proof.goal, "link_control");
    assert_eq!(proof.predicate_program_sha256, program_digest);
    assert_eq!(proof.predicate_definition_sha256, digest);
    assert_eq!(proof.first_hit_tick, Some(700));

    let invalid = ROUTE.replace(
        "program 2222222222222222222222222222222222222222222222222222222222222222",
        "program NOT-A-DIGEST",
    );
    assert!(
        Timeline::parse(&invalid)
            .unwrap_err()
            .to_string()
            .contains("64 lowercase hexadecimal")
    );

    assert!(timeline.proofs.iter().any(|proof| {
        proof.segment == "boot_fast"
            && proof.goal == "link_control"
            && timeline.segments["boot_fast"].parent == timeline.segments["boot_safe"].parent
    }));

    let duplicate = format!(
        "{ROUTE}\nproof boot_safe satisfies link_control program {} predicate {}",
        "2".repeat(64),
        "1".repeat(64)
    );
    assert!(
        Timeline::parse(&duplicate)
            .unwrap_err()
            .to_string()
            .contains("duplicate proof")
    );
}

#[test]
fn rejects_goal_proof_from_an_unrelated_segment() {
    let unrelated = ROUTE.replace(
        "proof boot_fast satisfies link_control",
        "proof exit_safe satisfies link_control",
    );
    let error = Timeline::parse(&unrelated).unwrap_err();
    assert!(error.to_string().contains("unrelated segment boot_safe"));
    assert!(
        error
            .to_string()
            .contains("reference segment or one of its siblings")
    );
}

#[test]
fn rejects_unpinned_parent_boundary_mismatch() {
    let mismatch = ROUTE.replace(
        "segment exit_repaired after boot_fast profile fsp103_to_fsp104 uses baseline fsp103_to_fsp104 starts control-rng1",
        "segment exit_repaired after boot_fast profile fsp103_to_fsp104 uses baseline fsp103_to_fsp104 starts wrong-parent-state",
    );
    let error = Timeline::parse(&mismatch).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("segment exit_repaired starts at wrong-parent-state")
    );
    assert!(
        error
            .to_string()
            .contains("parent boot_fast ends at control-rng1")
    );
}

#[test]
fn adding_a_sibling_does_not_change_a_pinned_lineage() {
    let source = ROUTE.replace(
        "segment boot_other_rng root",
        "segment another_boot root profile boot_to_fsp103 uses baseline boot_to_fsp103 starts clean-rng1 produces other\nsegment boot_other_rng root",
    );
    let timeline = Timeline::parse(&source).unwrap();
    let status = timeline.status(None, &BTreeMap::new()).unwrap();
    assert!(
        status
            .immutable_lineages
            .iter()
            .all(|lineage| !lineage.stale)
    );
    assert_eq!(status.immutable_lineages[0].steps[0], "boot_safe");
}

#[test]
fn workspace_marks_descendants_stale_and_rebase_creates_new_lineage() {
    let timeline = Timeline::parse(ROUTE).unwrap();
    let selections = BTreeMap::from([("boot_safe".into(), "boot_fast".into())]);
    let status = timeline.status(Some("main"), &selections).unwrap();
    let workspace = status.workspace.unwrap();
    assert_eq!(workspace.steps[0].state, WorkspaceStepState::Selected);
    assert_eq!(workspace.steps[1].state, WorkspaceStepState::Stale);
    assert!(!workspace.steps[1].rebase_compatible);

    let poisoned = timeline
        .rebase_compatible("main", &selections, "main_fast")
        .unwrap();
    assert!(poisoned.old_lineage_preserved);
    assert!(!poisoned.fully_compatible);
    assert_eq!(poisoned.stale_descendants, vec!["exit_safe"]);

    let repaired_selections = BTreeMap::from([
        ("boot_safe".into(), "boot_fast".into()),
        ("exit_safe".into(), "exit_repaired".into()),
    ]);
    let repair = timeline
        .rebase_compatible("main", &repaired_selections, "main_repaired")
        .unwrap();
    assert!(repair.fully_compatible);
    assert!(
        repair
            .authored_dsl
            .contains("continue main_repaired with exit_repaired after boot_fast@control-rng1")
    );
}

#[test]
fn rejects_cycles_bad_references_and_boundary_mismatches_with_lines() {
    let cycle = ROUTE.replace(
        "segment boot_safe root profile",
        "segment boot_safe after exit_safe profile",
    );
    assert!(
        Timeline::parse(&cycle)
            .unwrap_err()
            .to_string()
            .contains("cycle")
    );

    let unknown = ROUTE.replace("with exit_safe", "with exit_missing");
    let error = Timeline::parse(&unknown).unwrap_err();
    assert!(error.to_string().contains("unknown segment"));
    assert!(error.line.is_some());

    let mismatch = ROUTE.replace("after boot_safe@control-rng1", "after boot_safe@wrong");
    assert!(
        Timeline::parse(&mismatch)
            .unwrap_err()
            .to_string()
            .contains("boundary mismatch")
    );

    let off_lineage = ROUTE.replace(
        "branch rolls from main after boot_safe",
        "branch rolls from main after boot_other_rng",
    );
    assert!(
        Timeline::parse(&off_lineage)
            .unwrap_err()
            .to_string()
            .contains("is not reached by main")
    );
}

#[test]
fn parser_reports_quoted_token_diagnostics() {
    let error = Timeline::parse("timeline \"unterminated").unwrap_err();
    assert_eq!(error.line, Some(1));
    assert!(error.to_string().contains("unterminated"));
}

#[test]
fn segment_labels_are_unique_bounded_metadata_for_existing_segments() {
    for source in [
        "timeline bad\nlabel missing \"Unknown\"",
        "timeline bad\nsegment root root profile boot_to_fsp103 uses baseline boot_to_fsp103 starts clean produces end\nlabel root one\nlabel root two",
        "timeline bad\nsegment root root profile boot_to_fsp103 uses baseline boot_to_fsp103 starts clean produces end\nlabel root \"\"",
    ] {
        assert!(Timeline::parse(source).is_err());
    }
}

#[test]
fn rejects_the_removed_variant_entity() {
    let error = Timeline::parse(
        "timeline old\nvariant boot.safe uses baseline boot_to_fsp103 starts clean produces control",
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("unknown timeline statement \"variant\"")
    );
}

#[test]
fn parses_one_relative_predicate_program() {
    let timeline = Timeline::parse(ROUTE).unwrap();
    assert_eq!(
        timeline.predicate_program.as_deref(),
        Some(Path::new("route.milestones"))
    );

    let absolute = Timeline::parse("timeline bad\npredicate_program C:/bad/program").unwrap_err();
    assert!(absolute.to_string().contains("contained relative path"));
    let traversal = Timeline::parse("timeline bad\npredicate_program ../outside").unwrap_err();
    assert!(traversal.to_string().contains("contained relative path"));
    let windows_traversal =
        Timeline::parse("timeline bad\npredicate_program ..\\outside").unwrap_err();
    assert!(
        windows_traversal
            .to_string()
            .contains("contained relative path")
    );
    let duplicate =
        Timeline::parse("timeline bad\npredicate_program a\npredicate_program b").unwrap_err();
    assert!(
        duplicate
            .to_string()
            .contains("duplicate predicate_program")
    );
}

#[test]
fn compiles_referenced_predicates_allows_unused_definitions_and_checks_proofs() {
    let root = std::env::temp_dir().join(format!(
        "huntctl-timeline-milestones-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("route.milestones"),
        r#"milestones 1.0
milestone process_boot {
  phase pre_input
  when boundary.kind == "boot" && boundary.index == 0
}
milestone link_control {
  phase post_sim
  when stage.name == "F_SP103" && player.exists
}
milestone unused_probe {
  phase post_sim
  when player.exists
}
"#,
    )
    .unwrap();
    let timeline = Timeline::parse(
        r#"timeline route
predicate_program route.milestones
origin boot predicate process_boot
segment boot root profile boot_to_fsp103 uses baseline boot_to_fsp103 starts clean produces control
goal control on boot predicate link_control
continuation main starts root@clean
continue main with boot after root@clean
"#,
    )
    .unwrap();
    let compiled = timeline.compile_predicates(&root).unwrap().unwrap();
    assert_eq!(compiled.definitions.len(), 3);
    timeline.validate_artifacts(Some(&root)).unwrap();

    let program_digest = compiled
        .program_sha256
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let predicate_digest = compiled
        .definitions
        .iter()
        .find(|definition| definition.name == "link_control")
        .unwrap()
        .sha256
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let pinned = Timeline::parse(&format!(
        r#"timeline route
predicate_program route.milestones
origin boot predicate process_boot
segment boot root profile boot_to_fsp103 uses baseline boot_to_fsp103 starts clean produces control
goal control on boot predicate link_control
proof boot satisfies control program {program_digest} predicate {predicate_digest}
continuation main starts root@clean
continue main with boot after root@clean
"#
    ))
    .unwrap();
    pinned.validate_artifacts(Some(&root)).unwrap();

    let changed = fs::read_to_string(root.join("route.milestones"))
        .unwrap()
        .replace(
            "phase post_sim\n  when",
            "phase post_sim\n  stable 2\n  when",
        );
    fs::write(root.join("route.milestones"), changed).unwrap();
    assert!(
        pinned
            .validate_artifacts(Some(&root))
            .unwrap_err()
            .to_string()
            .contains("stale predicate source")
    );

    let missing = Timeline::parse(
        r#"timeline route
predicate_program route.milestones
segment boot root profile boot_to_fsp103 uses baseline boot_to_fsp103 starts clean produces control
goal control on boot predicate not_defined
continuation main starts root@clean
continue main with boot after root@clean
"#,
    )
    .unwrap();
    assert!(
        missing
            .compile_predicates(&root)
            .unwrap_err()
            .to_string()
            .contains("does not define")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn goal_predicate_sources_are_local_and_independently_identified() {
    let root = std::env::temp_dir().join(format!(
        "huntctl-timeline-owned-predicates-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("first.milestones"),
        "milestones 1.0\nmilestone first { phase post_sim when player.exists }\n",
    )
    .unwrap();
    fs::write(
        root.join("second.milestones"),
        "milestones 1.0\nmilestone second { phase post_sim when event.running }\n",
    )
    .unwrap();
    let timeline = Timeline::parse(
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
    timeline.validate_artifacts(Some(&root)).unwrap();
    let first_before = timeline
        .compile_goal_predicate(&root, "first_goal")
        .unwrap()
        .program_sha256;
    let second_before = timeline
        .compile_goal_predicate(&root, "second_goal")
        .unwrap()
        .program_sha256;
    fs::write(
        root.join("second.milestones"),
        "milestones 1.0\nmilestone second { phase post_sim stable 2 when event.running }\n",
    )
    .unwrap();
    assert_eq!(
        timeline
            .compile_goal_predicate(&root, "first_goal")
            .unwrap()
            .program_sha256,
        first_before
    );
    assert_ne!(
        timeline
            .compile_goal_predicate(&root, "second_goal")
            .unwrap()
            .program_sha256,
        second_before
    );

    fs::write(
        root.join("second.milestones"),
        "milestones 1.0\nmilestone second { phase post_sim when event.running }\nmilestone historical_coupling { phase post_sim when player.exists }\n",
    )
    .unwrap();
    assert!(
        timeline
            .compile_goal_predicate(&root, "second_goal")
            .unwrap_err()
            .to_string()
            .contains("exactly its own predicate")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn validates_git_tracked_tas_artifacts() {
    let root = std::env::temp_dir().join(format!("huntctl-timeline-tas-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("boot.tas"),
        "dusktape 1\nrate 30/1\nports 0x0f\nstate neutral {}\nframe neutral\n",
    )
    .unwrap();
    let timeline = Timeline::parse(
        r#"
timeline tas_route
segment boot_link root profile boot_to_fsp103 uses tas boot.tas starts clean produces control
continuation main starts root@clean
continue main with boot_link after root@clean
"#,
    )
    .unwrap();
    timeline.validate_artifacts(Some(&root)).unwrap();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn nested_subgraphs_are_structural_single_entry_single_exit_regions() {
    let timeline = Timeline::parse(
        r#"
timeline nested
segment a root profile boot_to_fsp103 uses baseline boot_to_fsp103 starts clean produces one
segment b after a profile boot_to_fsp103 uses baseline boot_to_fsp103 starts one produces two
segment c after b profile boot_to_fsp103 uses baseline boot_to_fsp103 starts two produces three
subgraph outer root entry a exit c
subgraph_label outer "Outer graph"
subgraph_member outer segment a
subgraph_member outer segment c
subgraph inner inside outer entry b exit b
subgraph_label inner "Inner graph"
subgraph_member inner segment b
"#,
    )
    .unwrap();
    assert_eq!(timeline.subgraphs["outer"].name, "Outer graph");
    assert_eq!(
        timeline.subgraph_segment_closure("outer"),
        ["a", "b", "c"].into_iter().map(str::to_owned).collect()
    );
    assert_eq!(
        timeline.subgraph_segment_closure("inner"),
        ["b"].into_iter().map(str::to_owned).collect()
    );
}

#[test]
fn subgraphs_reject_overlapping_or_disconnected_regions() {
    let overlap = Timeline::parse(
        r#"
timeline overlap
segment a root profile boot_to_fsp103 uses baseline boot_to_fsp103 starts clean produces one
subgraph first root entry a exit a
subgraph_member first segment a
subgraph second root entry a exit a
subgraph_member second segment a
"#,
    )
    .unwrap_err();
    assert!(overlap.to_string().contains("belongs directly to both"));

    let disconnected = Timeline::parse(
        r#"
timeline disconnected
segment a root profile boot_to_fsp103 uses baseline boot_to_fsp103 starts clean produces one
segment b root profile boot_to_fsp103 uses baseline boot_to_fsp103 starts clean produces two
subgraph broken root entry a exit b
subgraph_member broken segment a
subgraph_member broken segment b
"#,
    )
    .unwrap_err();
    assert!(disconnected.to_string().contains("second entry"));
}
