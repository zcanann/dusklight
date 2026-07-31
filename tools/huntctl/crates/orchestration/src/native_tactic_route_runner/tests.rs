use super::candidate_retention::*;
use super::goal_target::*;
use super::journal::*;
use super::macro_discovery::*;
use super::worker_pool::recorded_demonstration_chunks;
use super::worker_pool::*;
use super::*;
use crate::native_tactic_worker::TACTIC_CHECKPOINT_CACHE_ENTRIES;
use dusklight_learning::tactic_macro_promotion::TacticMacroEntryConditionCell;

#[test]
fn unassisted_learning_requires_declared_generous_discovery_capacity() {
    assert_eq!(
        unassisted_discovery_horizon_requirement(
            CampaignClass::FromScratchDiscovery,
            TacticProposalPolicy::Learned,
            false,
            false,
            131,
        ),
        Ok(Some(900))
    );
    assert!(
        unassisted_discovery_horizon_requirement(
            CampaignClass::LocalTasRefinement,
            TacticProposalPolicy::Learned,
            false,
            false,
            131,
        )
        .is_err()
    );
    assert_eq!(
        unassisted_discovery_horizon_requirement(
            CampaignClass::LocalTasRefinement,
            TacticProposalPolicy::Learned,
            true,
            false,
            131,
        ),
        Ok(None)
    );
    assert_eq!(
        unassisted_discovery_horizon_requirement(
            CampaignClass::LocalTasRefinement,
            TacticProposalPolicy::RandomValid,
            false,
            false,
            131,
        ),
        Ok(None)
    );
}

fn acquisition_with_expansion_count(expansion_count: u64) -> TacticFrontierAcquisition {
    TacticFrontierAcquisition {
        expansion_count,
        terminal: false,
        terminal_value_supported: false,
        achieved_goal_value_supported: false,
        goal_reachability_supported: false,
        reward: 0.0,
        best_mean_q: None,
        best_goal_progress_per_tick: None,
        predicted_terminal_ticks_to_go: None,
        predicted_total_terminal_ticks: None,
        exact_terminal_ticks_to_go: None,
        exact_total_terminal_ticks: None,
        maximum_ensemble_variance: None,
        generalized_nearest_distance: None,
        discovery_spatial_novelty: None,
        novelty_rank: 0,
        replayed_prefix_ticks: 0,
    }
}

#[test]
fn graph_branch_schedule_prioritizes_terminal_support_and_retains_discovery_cadence() {
    assert!(!campaign::should_schedule_branch(0, 8, false, false));
    assert!(!campaign::should_schedule_branch(7, 8, false, false));
    assert!(campaign::should_schedule_branch(8, 8, false, false));
    assert!(campaign::should_schedule_branch(256, 8, false, false));
    assert!(campaign::should_schedule_branch(3, 8, true, false));
    assert!(campaign::should_schedule_branch(3, 8, false, true));
    assert!((0..32).all(|decision| campaign::should_schedule_branch(decision, 8, false, true)));
    assert!(!campaign::prefer_root_for_periodic_branch(true, false));
    assert!(!campaign::prefer_root_for_periodic_branch(true, true));
    assert!(!campaign::prefer_root_for_periodic_branch(false, false));
    assert!(campaign::prefer_root_for_periodic_branch(false, true));
}

#[test]
fn demonstration_frontier_intervention_only_forces_the_first_expansion() {
    let unexpanded = acquisition_with_expansion_count(0);
    let revisited = acquisition_with_expansion_count(1);

    assert!(super::campaign::first_demonstration_intervention(
        true,
        false,
        Some(&unexpanded)
    ));
    assert!(!super::campaign::first_demonstration_intervention(
        true,
        false,
        Some(&revisited)
    ));
    assert!(!super::campaign::first_demonstration_intervention(
        true,
        true,
        Some(&unexpanded)
    ));
    assert!(!super::campaign::first_demonstration_intervention(
        false,
        false,
        Some(&unexpanded)
    ));
    assert!(!super::campaign::first_demonstration_intervention(
        true, false, None
    ));
}

#[test]
fn demonstration_chunks_preserve_the_bounded_authenticated_suffix() {
    let mut frames = Vec::new();
    for value in 0_i8..12 {
        let mut frame = InputFrame::default();
        frame.owned_ports = 1;
        frame.pads[0].stick_x = value;
        frames.push(frame);
    }
    let process_tape = InputTape {
        frames,
        ..InputTape::default()
    };

    let chunks = recorded_demonstration_chunks(&process_tape, 2, 3, 7).unwrap();
    assert_eq!(
        chunks
            .iter()
            .map(|chunk| chunk.frames.len())
            .collect::<Vec<_>>(),
        vec![3, 3, 1]
    );
    assert_eq!(
        chunks
            .iter()
            .flat_map(|chunk| chunk.frames.iter().cloned())
            .collect::<Vec<_>>(),
        process_tape.frames[2..9]
    );
    assert!(chunks.iter().all(|chunk| {
        chunk.boot == process_tape.boot
            && chunk.tick_rate_numerator == process_tape.tick_rate_numerator
            && chunk.tick_rate_denominator == process_tape.tick_rate_denominator
    }));
}

#[test]
fn demonstration_chunks_never_skip_branchable_interior_boundaries() {
    assert_eq!(maximum_demonstration_chunk_ticks(160).unwrap(), 4);
    assert_eq!(maximum_demonstration_chunk_ticks(3).unwrap(), 1);
    assert_eq!(maximum_demonstration_chunk_ticks(1_000).unwrap(), 4);
    assert!(maximum_demonstration_chunk_ticks(0).is_err());
}

#[test]
fn demonstration_chunks_reject_empty_or_detached_sources() {
    let process_tape = InputTape {
        frames: vec![InputFrame::default(); 4],
        ..InputTape::default()
    };

    assert!(recorded_demonstration_chunks(&process_tape, 0, 0, 4).is_err());
    assert!(recorded_demonstration_chunks(&process_tape, 0, 2, 0).is_err());
    assert!(recorded_demonstration_chunks(&process_tape, 5, 2, 4).is_err());
    assert!(recorded_demonstration_chunks(&process_tape, 4, 2, 4).is_err());
}

#[test]
fn retained_success_ranking_minimizes_ticks_and_breaks_ties_deterministically() {
    assert!(successful_route_rank_is_better(
        124,
        Digest([9; 32]),
        125,
        Digest([0; 32])
    ));
    assert!(!successful_route_rank_is_better(
        126,
        Digest([0; 32]),
        125,
        Digest([9; 32])
    ));
    assert!(successful_route_rank_is_better(
        125,
        Digest([1; 32]),
        125,
        Digest([2; 32])
    ));
    assert!(!successful_route_rank_is_better(
        125,
        Digest([2; 32]),
        125,
        Digest([1; 32])
    ));
}

#[test]
fn terminal_route_success_is_strictly_better_than_the_promotion_tick() {
    let source_frame = 506;
    assert_eq!(
        route_frames_first_hit_tick(source_frame + 125, source_frame),
        Some(124)
    );
    assert_eq!(
        route_frames_first_hit_tick(source_frame + 126, source_frame),
        Some(125)
    );
    assert_eq!(
        route_frames_first_hit_tick(source_frame, source_frame),
        None
    );
    assert!(route_frames_promote(source_frame + 125, source_frame, 125));
    assert!(!route_frames_promote(source_frame + 126, source_frame, 125));
    assert!(!route_frames_promote(source_frame, source_frame, 125));
}

#[test]
fn macro_validation_keeps_distinct_tapes_that_converge_on_one_fact_snapshot() {
    let shard = NativeEpisodeShard::decode(include_bytes!(
        "../../../../../../tests/fixtures/automation/native_episode_v28.dseps"
    ))
    .unwrap();
    let snapshot = FactSnapshot::from_native_learning(
        &shard.episodes[0].steps[0].pre_input,
        &[],
        None,
        Vec::new(),
    )
    .unwrap();
    let state_sha256 = snapshot.content_sha256().unwrap();
    let neutral_route = InputTape {
        frames: vec![dusklight_automation_contracts::tape::InputFrame::default()],
        ..InputTape::default()
    };
    let mut rolling_route = neutral_route.clone();
    rolling_route.frames[0].owned_ports = 1;
    rolling_route.frames[0].pads[0].buttons = 0x0100;
    let mut frontiers = BTreeMap::new();

    insert_tactic_macro_validation_frontier(
        &mut frontiers,
        TacticMacroValidationFrontier {
            seed: 7,
            state_sha256,
            snapshot: snapshot.clone(),
            route_tape: neutral_route.clone(),
        },
    )
    .unwrap();
    insert_tactic_macro_validation_frontier(
        &mut frontiers,
        TacticMacroValidationFrontier {
            seed: 7,
            state_sha256,
            snapshot: snapshot.clone(),
            route_tape: rolling_route,
        },
    )
    .unwrap();
    insert_tactic_macro_validation_frontier(
        &mut frontiers,
        TacticMacroValidationFrontier {
            seed: 7,
            state_sha256,
            snapshot,
            route_tape: neutral_route.clone(),
        },
    )
    .unwrap();

    assert_eq!(frontiers.len(), 2);
    assert!(
        frontiers
            .values()
            .any(|frontier| frontier.route_tape == neutral_route)
    );
}

fn journal_trace(decision_index: u64) -> NativeTacticDecisionTrace {
    serde_json::from_value(serde_json::json!({
        "decision_index": decision_index,
        "cumulative_wall_micros": (decision_index + 1) * 1_000,
        "episode": 2,
        "route_suffix_ticks": decision_index + 4,
        "selected_option_id": format!("move.{decision_index}"),
        "selection_reason": "epsilon",
        "selected_q": 1.5,
        "best_q": 2.0,
        "reward": 0.25,
        "reward_components": {
            "terminal_observed": false,
            "endpoint_novel": true,
            "duration_ticks": 4,
            "terminal_component": 0.0,
            "tick_cost_component": 0.0,
            "novelty_component": 0.25,
            "base_reward": 0.25,
            "potential": null,
            "training_reward": 0.25,
            "terminal_objective_unchanged": true,
            "promotion_authority": false
        },
        "goal_distance_before": 8.0,
        "goal_distance_after": 7.0,
        "terminal": false,
        "frontier_cells": 3,
        "visited_states": 4,
        "before": {
            "snapshot_sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "stage": "F_SP103",
            "room": 1,
            "layer": 0,
            "point": 0,
            "simulation_tick": 10,
            "tape_frame": 20,
            "player_position": [0.0, 1.0, 2.0],
            "player_velocity": [0.0, 0.0, 1.0],
            "player_procedure": 3,
            "player_contacts": 1,
            "event_running": false,
            "event_id": -1,
            "terminal_reached": false,
            "actor_count": 4,
            "same_room_actor_count": 3,
            "recent_option_id": null
        },
        "after": {
            "snapshot_sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "stage": "F_SP103",
            "room": 1,
            "layer": 0,
            "point": 0,
            "simulation_tick": 14,
            "tape_frame": 24,
            "player_position": [1.0, 1.0, 2.0],
            "player_velocity": [1.0, 0.0, 0.0],
            "player_procedure": 3,
            "player_contacts": 1,
            "event_running": false,
            "event_id": -1,
            "terminal_reached": false,
            "actor_count": 4,
            "same_room_actor_count": 3,
            "recent_option_id": format!("move.{decision_index}")
        },
        "measurements": [{"name": "goal_distance", "before": 8.0, "after": 7.0}],
        "applicable_tactics": [{
            "option_id": format!("move.{decision_index}"),
            "descriptor": null,
            "mean_q": 1.5,
            "ensemble_variance": 0.25,
            "selected": true
        }]
    }))
    .unwrap()
}

fn journal_record(decision_index: u64) -> NativeTacticDecisionRecord {
    let root_tape = StoredContentRef {
        kind: dusklight_evidence::content_store::ContentKind::InputTape,
        sha256: Digest([2; 32]),
    };
    let transition = StoredContentRef {
        kind: dusklight_evidence::content_store::ContentKind::TacticTransition,
        sha256: Digest([1; 32]),
    };
    decision_record(
        &journal_trace(decision_index),
        2,
        Digest([3; 32]),
        root_tape,
        None,
        Some(transition),
        None,
        Vec::new(),
    )
}

#[test]
fn new_seed_directories_share_content_while_legacy_local_stores_win() {
    let campaign_root = std::env::temp_dir().join(format!(
        "dusklight-tactic-shared-seed-content-{}",
        std::process::id()
    ));
    let seed_root = campaign_root.join("seed-000-1");
    let _ = fs::remove_dir_all(&campaign_root);
    fs::create_dir_all(&seed_root).unwrap();

    assert_eq!(
        tactic_content_store_path(&seed_root),
        campaign_root.join(NATIVE_TACTIC_CONTENT_STORE_DIRECTORY)
    );
    let legacy_local = seed_root.join(NATIVE_TACTIC_CONTENT_STORE_DIRECTORY);
    fs::create_dir_all(&legacy_local).unwrap();
    assert_eq!(tactic_content_store_path(&seed_root), legacy_local);

    fs::remove_dir_all(campaign_root).unwrap();
}

#[test]
fn tactic_decision_journal_round_trips_and_recovers_a_truncated_tail() {
    let root = std::env::temp_dir().join(format!(
        "dusklight-tactic-decision-journal-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let mut appender = TacticDecisionJournalAppender::open(&root).unwrap();
    assert_eq!(appender.next_decision_index(), 0);
    appender.append(&journal_record(0)).unwrap();
    appender.append(&journal_record(1)).unwrap();
    assert_eq!(appender.next_decision_index(), 2);
    assert!(appender.append(&journal_record(3)).is_err());
    drop(appender);
    let records = read_tactic_decision_records(&root).unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[1].decision_index, 1);

    let path = tactic_decision_journal_path(&root);
    OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(&[7, 8, 9])
        .unwrap();
    assert_eq!(read_tactic_decision_records(&root).unwrap().len(), 2);
    let mut resumed = TacticDecisionJournalAppender::open(&root).unwrap();
    assert_eq!(resumed.next_decision_index(), 2);
    resumed.append(&journal_record(2)).unwrap();
    assert_eq!(read_tactic_decision_records(&root).unwrap().len(), 3);

    let mut bytes = fs::read(&path).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    fs::write(&path, bytes).unwrap();
    assert!(read_tactic_decision_records(&root).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn tactic_decision_journal_compacts_reference_records_without_rewriting_segments() {
    let root = std::env::temp_dir().join(format!(
        "dusklight-tactic-journal-compaction-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    for decision_index in 0..NATIVE_TACTIC_DECISION_COMPACTION_RECORDS {
        append_tactic_decision_record(&root, &journal_record(decision_index)).unwrap();
    }
    assert_eq!(
        fs::metadata(tactic_decision_journal_path(&root))
            .unwrap()
            .len() as usize,
        NATIVE_TACTIC_DECISION_JOURNAL_HEADER_SIZE
    );
    let first_segment = tactic_decision_segment_paths(&root).unwrap();
    assert_eq!(first_segment.len(), 1);
    let first_segment_bytes = fs::read(&first_segment[0]).unwrap();
    let stale_active = (0..NATIVE_TACTIC_DECISION_COMPACTION_RECORDS)
        .map(journal_record)
        .collect::<Vec<_>>();
    fs::write(
        tactic_decision_journal_path(&root),
        encode_tactic_decision_journal(&stale_active).unwrap(),
    )
    .unwrap();
    assert_eq!(
        read_tactic_decision_records(&root).unwrap().len() as u64,
        NATIVE_TACTIC_DECISION_COMPACTION_RECORDS
    );
    fs::remove_file(tactic_decision_journal_path(&root)).unwrap();
    assert!(has_tactic_decision_journal(&root));
    assert_eq!(
        read_tactic_decision_records(&root).unwrap().len() as u64,
        NATIVE_TACTIC_DECISION_COMPACTION_RECORDS
    );

    append_tactic_decision_record(
        &root,
        &journal_record(NATIVE_TACTIC_DECISION_COMPACTION_RECORDS),
    )
    .unwrap();
    compact_tactic_decision_journal(&root).unwrap();
    let segments = tactic_decision_segment_paths(&root).unwrap();
    assert_eq!(segments.len(), 2);
    assert_eq!(fs::read(&first_segment[0]).unwrap(), first_segment_bytes);
    assert_eq!(
        read_tactic_decision_records(&root).unwrap().len() as u64,
        NATIVE_TACTIC_DECISION_COMPACTION_RECORDS + 1
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn journal_projects_graph_and_materializes_routes_from_content_objects() {
    use dusklight_control::option_execution::{
        OptionCondition, OptionEndReason, OptionExecution, OptionType, TapeRange,
    };
    use dusklight_evidence::native_episode_shard::NativeObservationPhase;
    use dusklight_learning::fact_snapshot::{FactSnapshot, FactTerminalReason};
    use dusklight_learning::option_transition::OptionTransitionSample;

    let root = std::env::temp_dir().join(format!(
        "dusklight-tactic-journal-route-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let shard = NativeEpisodeShard::decode(include_bytes!(
        "../../../../../../tests/fixtures/automation/native_episode_v28.dseps"
    ))
    .unwrap();
    let native = &shard.episodes[0].steps[0];
    let mut before =
        FactSnapshot::from_native_learning(&native.pre_input, &[], None, Vec::new()).unwrap();
    let mut next_boundary = native.post_simulation.clone();
    next_boundary.phase = NativeObservationPhase::PreInput;
    next_boundary.simulation_tick += 1;
    next_boundary.tape_frame += 1;
    let mut after = FactSnapshot::from_native_learning(
        &next_boundary,
        &[native.pre_input.clone()],
        None,
        Vec::new(),
    )
    .unwrap();
    before.terminal.configured = Some(true);
    before.terminal.reached = Some(false);
    before.terminal.reason = FactTerminalReason::None;
    after.terminal.configured = Some(true);
    after.terminal.reached = Some(false);
    after.terminal.reason = FactTerminalReason::None;
    let mut route = InputTape {
        frames: vec![
            dusklight_automation_contracts::tape::InputFrame::default();
            after.tape_frame as usize
        ],
        ..InputTape::default()
    };
    after.tape_frame = route.frames.len() as u64;
    route.frames[before.tape_frame as usize] =
        dusklight_automation_contracts::tape::InputFrame::default();
    let execution = OptionExecution::capture(
        "wait".into(),
        OptionType::Neutral,
        BTreeMap::new(),
        1,
        1,
        OptionCondition::DurationElapsed,
        Vec::new(),
        OptionEndReason::Completed,
        &route,
        TapeRange {
            start_frame: before.tape_frame,
            end_frame_exclusive: after.tape_frame,
        },
    )
    .unwrap();
    let root_tape = InputTape {
        boot: route.boot.clone(),
        tick_rate_numerator: route.tick_rate_numerator,
        tick_rate_denominator: route.tick_rate_denominator,
        frames: route.frames[..before.tape_frame as usize].to_vec(),
    };
    let root_checkpoint_sha256 = Digest([9; 32]);
    let source_checkpoint_sha256 = route_checkpoint(root_checkpoint_sha256, &root_tape).unwrap();
    let next_checkpoint_sha256 = route_checkpoint(root_checkpoint_sha256, &route).unwrap();
    let transition = OptionTransitionSample::capture(
        Digest([1; 32]),
        source_checkpoint_sha256,
        next_checkpoint_sha256,
        before.clone(),
        after.clone(),
        execution,
        &route,
        0.25,
        false,
        |facts| Ok::<_, &'static str>(vec![facts.player.position_f32_bits[0] as f32]),
    )
    .unwrap();
    let store = TacticQContentStore::initialize(tactic_content_store_path(&root)).unwrap();
    let root_ref = store.store_tape(&root_tape).unwrap();
    let mut trace = journal_trace(0);
    trace.reward_components.duration_ticks = 1;
    let proposal_trace = NativeTacticProposalTrace {
        execution_plan_sha256: Digest::ZERO,
        option_id: transition.value_sample.action.option_id.clone(),
        selection_reason: trace.selection_reason,
        predicted_goal_progress_per_tick: Some(1.25),
        reachability_nearest_distance: Some(0.5),
        reward: transition.value_sample.reward,
        reward_components: trace.reward_components.clone(),
        realized_ticks: transition.execution.duration.realized_ticks,
        root_route_ticks: 1,
        emitted_tape_sha256: transition.value_sample.realized_tape_sha256,
        terminal: transition.value_sample.terminal,
        goal_distance_after: trace.goal_distance_after,
        after_snapshot_sha256: transition.after_state_sha256,
        retained: true,
    };
    trace.proposal_batch = vec![proposal_trace.clone()];
    let record = decision_record(
        &trace,
        2,
        root_checkpoint_sha256,
        root_ref,
        Some(root_ref),
        None,
        Some(transition.clone()),
        vec![NativeTacticProposalRecord {
            trace: proposal_trace,
            component: None,
            transition: None,
            inline_transition: Some(transition.clone()),
        }],
    );
    let mut detached = record.clone();
    detached.proposal_batch[0].trace.emitted_tape_sha256 = Digest([0xff; 32]);
    assert!(project_tactic_decision_record(&store, detached).is_err());
    let mut detached = record.clone();
    let mut sibling = detached.proposal_batch[0].clone();
    detached.proposal_batch[0].trace.retained = false;
    sibling.trace.retained = true;
    detached.proposal_batch.push(sibling);
    assert!(project_tactic_decision_record(&store, detached).is_err());
    let mut detached = record.clone();
    let mut component_frame = InputFrame::default();
    component_frame.owned_ports = 1;
    detached.proposal_batch[0].component = Some(
        TacticMacroComponent::from_catalog_entry(
            &TacticCatalogEntry::new(
                "family/detached-component",
                TacticAssetSource::RecordedTape(InputTape {
                    frames: vec![component_frame],
                    ..InputTape::default()
                }),
            )
            .unwrap(),
        )
        .unwrap(),
    );
    assert!(project_tactic_decision_record(&store, detached).is_err());
    append_tactic_decision_record(&root, &record).unwrap();

    let projected_trace = read_tactic_decision_journal(&root).unwrap();
    assert_eq!(projected_trace[0].proposal_batch, trace.proposal_batch);
    assert_eq!(
        projected_trace[0].applicable_tactics,
        trace.applicable_tactics
    );
    assert_eq!(
        projected_trace[0].cumulative_wall_micros,
        trace.cumulative_wall_micros
    );
    let graph = project_tactic_decision_graph(&root).unwrap().unwrap();
    assert!(graph.root_connected);
    assert_eq!(graph.nodes.len(), 2);
    assert_eq!(graph.edges.len(), 1);
    assert_eq!(graph.edges[0].option_id, "wait");
    let diagnostics = project_tactic_decision_diagnostics(&root).unwrap().unwrap();
    assert_eq!(diagnostics.replay_rows, 1);
    assert_eq!(diagnostics.frontier_cells, 1);
    assert_eq!(diagnostics.logical_frontier_records, 2);
    assert_eq!(diagnostics.directly_restorable_native_frontiers, 0);
    assert_eq!(diagnostics.replay_only_frontiers, 1);
    assert_eq!(diagnostics.unique_selected_actions, 1);
    assert!(!diagnostics.frontier_lost_root_connectivity);
    let materialized = materialize_tactic_decision_route(&root, 0).unwrap();
    assert_eq!(materialized, route);

    let mut alternate_route = route.clone();
    alternate_route.frames[before.tape_frame as usize].pads[0].stick_x = 64;
    let mut alternate_after = after.clone();
    alternate_after.player.position_f32_bits[0] ^= 1;
    let alternate_execution = OptionExecution::capture(
        "alternate".into(),
        OptionType::Custom("alternate".into()),
        BTreeMap::new(),
        1,
        1,
        OptionCondition::DurationElapsed,
        Vec::new(),
        OptionEndReason::Completed,
        &alternate_route,
        TapeRange {
            start_frame: before.tape_frame,
            end_frame_exclusive: alternate_after.tape_frame,
        },
    )
    .unwrap();
    let alternate_transition = OptionTransitionSample::capture(
        Digest([1; 32]),
        source_checkpoint_sha256,
        route_checkpoint(root_checkpoint_sha256, &alternate_route).unwrap(),
        before,
        alternate_after.clone(),
        alternate_execution,
        &alternate_route,
        0.5,
        false,
        |facts| Ok::<_, &'static str>(vec![facts.player.position_f32_bits[0] as f32]),
    )
    .unwrap();
    let mut child_route = alternate_route.clone();
    child_route
        .frames
        .push(dusklight_automation_contracts::tape::InputFrame::default());
    let child_before = alternate_after;
    let mut child_after = child_before.clone();
    child_after.simulation_tick += 1;
    child_after.tape_frame += 1;
    child_after.state_identity[0] ^= 1;
    let child_execution = OptionExecution::capture(
        "child".into(),
        OptionType::Custom("child".into()),
        BTreeMap::new(),
        1,
        1,
        OptionCondition::DurationElapsed,
        Vec::new(),
        OptionEndReason::Completed,
        &child_route,
        TapeRange {
            start_frame: child_before.tape_frame,
            end_frame_exclusive: child_after.tape_frame,
        },
    )
    .unwrap();
    let child_transition = OptionTransitionSample::capture(
        Digest([1; 32]),
        alternate_transition.next_checkpoint_sha256,
        route_checkpoint(root_checkpoint_sha256, &child_route).unwrap(),
        child_before,
        child_after,
        child_execution,
        &child_route,
        0.75,
        false,
        |facts| Ok::<_, &'static str>(vec![facts.player.position_f32_bits[0] as f32]),
    )
    .unwrap();
    let proposal_transitions = vec![
        vec![transition.clone(), alternate_transition],
        vec![child_transition.clone()],
    ];
    let mut parents = BTreeMap::new();
    for (decision_index, proposals) in proposal_transitions.iter().enumerate() {
        for (proposal_index, proposal) in proposals.iter().enumerate() {
            parents.insert(
                (proposal.next_checkpoint_sha256, proposal.after_state_sha256),
                (decision_index, proposal_index),
            );
        }
    }
    let branched_route = materialize_journal_route(
        1,
        &child_transition,
        &root_tape,
        root_checkpoint_sha256,
        (
            transition.source_checkpoint_sha256,
            transition.before_state_sha256,
        ),
        &parents,
        &proposal_transitions,
        None,
    )
    .unwrap();
    assert_eq!(branched_route, child_route);
    let anchored_route = materialize_journal_route(
        1,
        &child_transition,
        &root_tape,
        root_checkpoint_sha256,
        (
            transition.source_checkpoint_sha256,
            transition.before_state_sha256,
        ),
        &BTreeMap::new(),
        &[],
        Some(&alternate_route),
    )
    .unwrap();
    assert_eq!(anchored_route, child_route);
    assert!(
        materialize_journal_route(
            1,
            &child_transition,
            &root_tape,
            root_checkpoint_sha256,
            (
                transition.source_checkpoint_sha256,
                transition.before_state_sha256,
            ),
            &BTreeMap::new(),
            &[],
            Some(&root_tape),
        )
        .is_err()
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn route_reward_contains_only_terminal_success_and_native_tick_cost() {
    let reward = route_tactic_base_reward_spec();
    let values = route_option_value_config(Digest([42; 32]));

    assert_eq!(reward.tick_cost, ROUTE_TACTIC_TICK_COST);
    assert!(reward.terminal_reward > reward.tick_cost * 1_024.0);
    assert_eq!(reward.per_tick_discount, 1.0);
    assert_eq!(values.fitted_q.discount, ROUTE_TACTIC_VALUE_DISCOUNT);
    assert!(values.fitted_q.discount > 0.995);
    assert!(values.fitted_q.discount < 1.0);
    assert_eq!(reward.novelty_reward, 0.0);
    assert!(reward.potential.is_none());
    assert!(reward.motion_cost.is_none());
}

#[test]
fn root_probe_cache_request_binds_the_authenticated_source_with_the_sealed_capacity() {
    let mut batch = NativeSuffixBatch {
        schema: NATIVE_SUFFIX_BATCH_SCHEMA.into(),
        source_frame: 0,
        source_boundary_fingerprint: "0".repeat(32),
        checkpoint_validation: NativeCheckpointValidation {
            kind: "recorded_replay_window".into(),
            ticks: 1,
        },
        maximum_ticks: 1,
        verify_state_hashes: false,
        checkpoint_cache: None,
        candidates: Vec::new(),
    };
    attach_root_probe_checkpoint_cache(&mut batch, 384 * 1024 * 1024);
    let request = batch.checkpoint_cache.unwrap();

    assert_eq!(batch.schema, NATIVE_CACHED_SUFFIX_BATCH_SCHEMA);
    assert_eq!(request.capacity_bytes, 384 * 1024 * 1024);
    assert_eq!(request.capacity_entries, TACTIC_CHECKPOINT_CACHE_ENTRIES);
    assert_eq!(request.source_identity, None);
    assert_eq!(request.source_route_ticks, 0);
    assert!(!request.retain_candidate_checkpoints);
    assert!(!request.retain_live_endpoint);
}

#[test]
fn terminal_wall_median_is_overflow_safe_for_even_and_odd_seed_counts() {
    assert_eq!(median_sorted_wall_micros(&[]), None);
    assert_eq!(median_sorted_wall_micros(&[10]), Some(10));
    assert_eq!(median_sorted_wall_micros(&[10, 20, 30]), Some(20));
    assert_eq!(median_sorted_wall_micros(&[10, 21]), Some(15));
    assert_eq!(
        median_sorted_wall_micros(&[u64::MAX - 1, u64::MAX]),
        Some(u64::MAX - 1)
    );
}

#[test]
fn candidate_budget_caps_total_decisions_across_all_seed_lanes() {
    assert!(planned_decisions_fit_candidate_budget(256, 4, 1_024));
    assert!(!planned_decisions_fit_candidate_budget(257, 4, 1_024));
    assert!(!planned_decisions_fit_candidate_budget(
        u64::MAX,
        2,
        u64::MAX
    ));
}

#[test]
fn horizon_fit_uses_the_selected_tactic_duration() {
    assert!(selected_tactic_fits_horizon(88, 8, 160));
    assert!(selected_tactic_fits_horizon(152, 8, 160));
    assert!(!selected_tactic_fits_horizon(88, 80, 160));
    assert!(!selected_tactic_fits_horizon(u64::MAX, 1, 160));
}

#[test]
fn throughput_rates_use_measured_wall_time_and_sum_seed_phases() {
    let seed = NativeTacticSeedResult {
        execution_plan_sha256: Digest::ZERO,
        seed: 7,
        terminal_discovered: false,
        best_authenticated_tick: None,
        first_terminal_decision_index: None,
        time_to_first_terminal_micros: None,
        wall_budget_reached: false,
        stop_reasons: vec![NativeTacticSeedStopReason::DecisionBudgetReached],
        success: false,
        decisions: 4,
        episodes: 2,
        native_ticks: 30,
        replay_rows: 4,
        training_replay_rows: 12,
        imported_training_replay_rows: 0,
        duplicate_training_transitions: 4,
        censored_training_transitions: 0,
        learner_updates: 3,
        replay_sharing: NativeTacticReplaySharingTelemetry::default(),
        visited_states: 3,
        useful_decisions: 2,
        unique_useful_graph_expansions: 6,
        native_restore_accounting: NativeTacticRestoreAccounting::default(),
        timing: NativeTacticRouteTiming {
            wall_micros: 2_000_000,
            process_launch_micros: 30_000,
            tactic_selection_micros: 10,
            checkpoint_branching_micros: 20,
            tactic_execution_micros: 1_000_000,
            native_simulation_micros: 900_000,
            ipc_and_result_transport_micros: 40_000,
            native_observation_capture_micros: 50_000,
            native_corpus_encoding_micros: 60_000,
            rust_state_extraction_micros: 70_000,
            tactic_preparation_and_fact_extraction_micros: 100_000,
            model_update_micros: 200_000,
            evidence_projection_and_persistence_micros: 300_000,
            evidence_projection_micros: 100_000,
            persistence_micros: 200_000,
            persistence_breakdown: Some(NativeTacticPersistenceTiming {
                source_tape_micros: 10_000,
                recovery_checkpoint_micros: 20_000,
                decision_journal_micros: 30_000,
                replay_content_micros: 0,
                replay_publication_micros: 40_000,
                lease_resolution_micros: 10_000,
                recovery_prune_micros: 10_000,
                retained_terminal_micros: 10_000,
                finalization_micros: 20_000,
                unattributed_micros: 50_000,
            }),
            orchestration_micros: 50_000,
            result_validation_and_fact_extraction_micros: 20_000,
            campaign_admission_micros: 20_000,
            campaign_admission_breakdown: Some(NativeTacticCampaignAdmissionTiming {
                terminal_projection_micros: 1_000,
                batch_graph_admission_micros: 2_000,
                next_action_catalog_micros: 3_000,
                selected_outcome_retention_micros: 4_000,
                frontier_retention_micros: 5_000,
                unattributed_micros: 5_000,
            }),
            graph_admission_micros: 15_000,
            reporting_micros: 25_000,
            ..NativeTacticRouteTiming::default()
        },
        selection_counts: BTreeMap::new(),
        diagnostics: None,
        final_checkpoint: "checkpoint.dtqz".into(),
        state_graph_sha256: Digest([7; 32]),
        useful_graph_expansion_set_sha256: Digest([8; 32]),
        graph_metrics: None,
        best_terminal_state_sha256: None,
        best_terminal_route_checkpoint_sha256: None,
        best_terminal_tape: None,
        best_terminal_result: None,
        successful_tape: None,
        final_result: None,
        trace: Vec::new(),
    };
    let timing = aggregate_route_timing(&[seed]);

    assert_eq!(timing.useful_decisions_per_second_millionths, 1_000_000);
    assert_eq!(
        timing.unique_useful_graph_expansions_per_second_millionths,
        3_000_000
    );
    assert_eq!(timing.native_ticks_per_second_millionths, 15_000_000);
    assert_eq!(timing.episodes_per_second_millionths, 1_000_000);
    assert_eq!(timing.native_simulation_micros, 900_000);
    assert_eq!(timing.evidence_projection_micros, 100_000);
    assert_eq!(timing.persistence_micros, 200_000);
    assert_eq!(
        timing
            .persistence_breakdown
            .expect("fixture has persistence attribution")
            .total_micros(),
        timing.persistence_micros
    );
    assert!(timing.persistence_attribution_is_valid());
    let mut detached_timing = timing.clone();
    detached_timing.persistence_micros += 1;
    assert!(!detached_timing.persistence_attribution_is_valid());
    assert_eq!(timing.orchestration_micros, 50_000);
    assert_eq!(timing.result_validation_and_fact_extraction_micros, 20_000);
    assert_eq!(timing.campaign_admission_micros, 20_000);
    assert_eq!(
        timing.campaign_admission_breakdown,
        Some(NativeTacticCampaignAdmissionTiming {
            terminal_projection_micros: 1_000,
            batch_graph_admission_micros: 2_000,
            next_action_catalog_micros: 3_000,
            selected_outcome_retention_micros: 4_000,
            frontier_retention_micros: 5_000,
            unattributed_micros: 5_000,
        })
    );
    assert_eq!(timing.process_launch_micros, 30_000);
    assert_eq!(timing.ipc_and_result_transport_micros, 40_000);
    assert_eq!(timing.native_observation_capture_micros, 50_000);
    assert_eq!(timing.native_corpus_encoding_micros, 60_000);
    assert_eq!(timing.rust_state_extraction_micros, 70_000);
    assert_eq!(timing.graph_admission_micros, 15_000);
    assert_eq!(timing.reporting_micros, 25_000);
}

#[test]
fn legacy_persistence_timing_round_trips_without_new_zero_fields() {
    let legacy = serde_json::json!({
        "source_tape_micros": 1,
        "recovery_checkpoint_micros": 2,
        "decision_journal_micros": 3,
        "replay_publication_micros": 4,
        "lease_resolution_micros": 5,
        "recovery_prune_micros": 6,
        "retained_terminal_micros": 7,
        "finalization_micros": 8,
        "unattributed_micros": 9
    });
    let timing: NativeTacticPersistenceTiming = serde_json::from_value(legacy.clone()).unwrap();

    assert_eq!(timing.replay_content_micros, 0);
    assert_eq!(serde_json::to_value(timing).unwrap(), legacy);
}

#[test]
fn restore_accounting_aggregates_cost_cache_memory_and_transition_yield() {
    let mut first = NativeTacticRestoreAccounting {
        native_requests: 4,
        authenticated_root_restore_requests: 1,
        direct_process_local_restore_requests: 2,
        direct_process_local_continuation_requests: 1,
        direct_restore_fallback_replays: 1,
        prefix_materializations: 1,
        replayed_prefix_ticks: 40,
        restore_samples: 4,
        restore_micros: 30,
        authenticated_root_restore_micros: 10,
        direct_process_local_restore_micros: 20,
        replay_restore_micros: 100,
        cache_hits: 2,
        cache_misses: 1,
        cache_evictions: 3,
        checkpoint_capture_attempts: 3,
        checkpoint_capture_successes: 2,
        checkpoint_capture_micros: 50,
        live_endpoint_retention_attempts: 1,
        live_endpoint_retention_successes: 1,
        live_endpoint_retention_nanos: 100,
        peak_resident_entries: 2,
        peak_resident_bytes: 600,
        peak_resident_checkpoint_bytes: 590,
        peak_resident_host_snapshot_bytes: 10,
        peak_live_endpoint_entries: 1,
        peak_live_endpoint_host_snapshot_bytes: 64,
        proposal_transitions: 2,
        useful_transitions: 1,
        ..NativeTacticRestoreAccounting::default()
    };
    first.refresh_rates();
    let mut second = NativeTacticRestoreAccounting {
        native_requests: 1,
        authenticated_root_restore_requests: 1,
        direct_restore_fallback_replays: 2,
        restore_samples: 1,
        restore_micros: 30,
        authenticated_root_restore_micros: 30,
        replay_restore_micros: 50,
        cache_misses: 1,
        cache_evictions: 4,
        checkpoint_capture_attempts: 1,
        checkpoint_capture_successes: 1,
        checkpoint_capture_micros: 20,
        live_endpoint_retention_attempts: 1,
        live_endpoint_retention_successes: 1,
        live_endpoint_retention_nanos: 50,
        peak_resident_entries: 1,
        peak_resident_bytes: 300,
        peak_resident_checkpoint_bytes: 295,
        peak_resident_host_snapshot_bytes: 5,
        peak_live_endpoint_entries: 1,
        peak_live_endpoint_host_snapshot_bytes: 32,
        proposal_transitions: 2,
        useful_transitions: 2,
        ..NativeTacticRestoreAccounting::default()
    };
    second.refresh_rates();

    first.merge(&second);

    assert_eq!(first.native_requests, 5);
    assert_eq!(first.restore_samples, 5);
    assert_eq!(first.restore_micros, 60);
    assert_eq!(first.authenticated_root_restore_micros, 40);
    assert_eq!(first.direct_process_local_restore_micros, 20);
    assert_eq!(first.replay_restore_micros, 150);
    assert_eq!(first.mean_restore_micros, 12);
    assert_eq!(first.direct_restore_request_rate_per_million, 600_000);
    assert_eq!(first.direct_restore_fallback_replays, 3);
    assert_eq!(first.cache_hit_rate_per_million, 500_000);
    assert_eq!(first.cache_evictions, 7);
    assert_eq!(first.replayed_prefix_ticks, 40);
    assert_eq!(first.live_endpoint_retention_attempts, 2);
    assert_eq!(first.live_endpoint_retention_successes, 2);
    assert_eq!(first.live_endpoint_retention_nanos, 150);
    assert_eq!(first.peak_live_endpoint_entries, 1);
    assert_eq!(first.peak_live_endpoint_host_snapshot_bytes, 64);
    assert_eq!(first.peak_resident_bytes, 600);
    assert_eq!(first.proposal_transitions, 4);
    assert_eq!(first.useful_transitions, 3);
    assert_eq!(first.useful_transitions_per_restore_millionths, 600_000);
}

#[test]
fn coordinator_results_are_projected_in_seed_order() {
    let seed_count = 11;
    let mut report_order = vec![8, 0, 10, 3, 5, 1, 9, 2, 7, 6, 4];
    report_order.sort_unstable();
    assert_eq!(report_order, (0..seed_count).collect::<Vec<_>>());
}

#[test]
fn evaluated_tick_accounting_includes_every_proposal() {
    let mut trace = journal_trace(0);
    assert_eq!(decision_evaluated_ticks(&trace), 4);
    trace.proposal_batch = [6, 14, 40]
        .into_iter()
        .enumerate()
        .map(|(index, realized_ticks)| NativeTacticProposalTrace {
            execution_plan_sha256: Digest::ZERO,
            option_id: format!("proposal-{index}"),
            selection_reason: TacticSelectionReason::BatchDiversity,
            predicted_goal_progress_per_tick: None,
            reachability_nearest_distance: None,
            reward: index as f32,
            reward_components: trace.reward_components.clone(),
            realized_ticks,
            root_route_ticks: u64::from(realized_ticks),
            emitted_tape_sha256: Digest([index as u8 + 1; 32]),
            terminal: index == 1,
            goal_distance_after: 7.0,
            after_snapshot_sha256: Digest([index as u8 + 1; 32]),
            retained: index == 1,
        })
        .collect();
    assert_eq!(decision_evaluated_ticks(&trace), 60);
}

#[test]
fn root_episode_slots_do_not_skip_frontier_rotation_rounds() {
    let frontier_rounds = (1..=11)
        .filter(|episode| episode % 4 != 0)
        .map(frontier_sampling_round)
        .collect::<Vec<_>>();
    assert_eq!(frontier_rounds, (0..9).collect::<Vec<_>>());
}

#[test]
fn tactic_macro_validation_waits_for_independent_seed_support() {
    assert!(!tactic_macro_promotion_has_seed_support(&[]));
    assert!(!tactic_macro_promotion_has_seed_support(&[104_729]));
    assert!(!tactic_macro_promotion_has_seed_support(&[
        104_729, 104_729
    ]));
    assert!(tactic_macro_promotion_has_seed_support(&[104_729, 130_363]));
}

#[test]
fn connected_macro_needs_repeated_occurrences_not_internal_steps() {
    let tape = InputTape {
        frames: vec![
            InputFrame {
                owned_ports: 1,
                ..InputFrame::default()
            };
            8
        ],
        ..InputTape::default()
    };
    let source = |seed: u64, state: u8, transition: u8| MacroSourceProvenance {
        seed,
        frontier_state_sha256: Digest([state; 32]),
        transition_sha256s: vec![
            Digest([transition; 32]),
            Digest([transition.saturating_add(1); 32]),
        ],
        entry: MacroEntryObservation {
            stage: "F_SP103".into(),
            room: 1,
            player_procedure: Some(3),
            player_contacts: Some(1),
            goal_distance_f32_bits: (100.0 + f32::from(state)).to_bits(),
        },
    };
    let component = |option_id: &str| {
        TacticMacroComponent::from_catalog_entry(
            &TacticCatalogEntry::new(option_id, TacticAssetSource::RecordedTape(tape.clone()))
                .unwrap(),
        )
        .unwrap()
    };
    let components = vec![
        component("family/primitive/a"),
        component("family/primitive/b"),
    ];
    let occurrences = |sources: Vec<MacroSourceProvenance>| {
        let sources = sources
            .into_iter()
            .map(|source| (source.transition_sha256s.clone(), source))
            .collect::<BTreeMap<_, _>>();
        BTreeMap::from([(
            tape.encode().unwrap(),
            (tape.clone(), components.clone(), sources),
        )])
    };

    assert!(
        connected_macro_candidates(occurrences(vec![source(11, 1, 3)]))
            .unwrap()
            .is_empty()
    );
    let candidates =
        connected_macro_candidates(occurrences(vec![source(11, 1, 3), source(13, 2, 4)])).unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].components, components);
    assert_eq!(candidates[0].sources.len(), 2);
    assert_eq!(
        candidates[0]
            .sources
            .iter()
            .map(|source| source.frontier_state_sha256)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([Digest([1; 32]), Digest([2; 32])])
    );
}

#[test]
fn tactic_macro_entry_conditions_admit_nearby_held_out_states_only() {
    let shard = NativeEpisodeShard::decode(include_bytes!(
        "../../../../../../tests/fixtures/automation/native_episode_v28.dseps"
    ))
    .unwrap();
    let snapshot = FactSnapshot::from_native_learning(
        &shard.episodes[0].steps[0].pre_input,
        &[],
        None,
        Vec::new(),
    )
    .unwrap();
    let encoder = GoalConditionedTacticFeatureEncoder::new([0.0, 0.0, 0.0]).unwrap();
    let goal_distance = encoder.encode(&snapshot).unwrap()[encoder.goal_distance_feature()];
    let condition = dusklight_learning::tactic_macro_promotion::TacticMacroEntryCondition {
        cells: vec![TacticMacroEntryConditionCell {
            stage: snapshot.world.stage.clone(),
            room: snapshot.world.room,
            player_procedure: snapshot.player.procedure,
            player_contacts: snapshot.player.contacts,
            minimum_goal_distance: goal_distance,
            maximum_goal_distance: goal_distance,
        }],
    };
    let frontier = |snapshot: FactSnapshot| TacticMacroValidationFrontier {
        seed: 11,
        state_sha256: snapshot.content_sha256().unwrap(),
        snapshot,
        route_tape: InputTape {
            frames: vec![InputFrame::default()],
            ..InputTape::default()
        },
    };

    assert_eq!(
        tactic_macro_entry_distance(&condition, &frontier(snapshot.clone()), &encoder).unwrap(),
        Some(0.0)
    );
    let mut wrong_room = snapshot.clone();
    wrong_room.world.room = wrong_room.world.room.saturating_add(1);
    assert_eq!(
        tactic_macro_entry_distance(&condition, &frontier(wrong_room), &encoder).unwrap(),
        None
    );
    let mut too_far = snapshot;
    too_far.player.position_f32_bits[0] =
        (f32::from_bits(too_far.player.position_f32_bits[0]) + 1024.0).to_bits();
    assert_eq!(
        tactic_macro_entry_distance(&condition, &frontier(too_far), &encoder).unwrap(),
        None
    );
}

#[test]
fn promoted_macro_reuse_accepts_only_an_exact_realized_prefix() {
    let first = InputFrame {
        owned_ports: 1,
        ..InputFrame::default()
    };
    let mut second = first.clone();
    second.pads[0].buttons = 0x0100;
    let candidate = InputTape {
        frames: vec![first.clone(), second.clone()],
        ..InputTape::default()
    };

    assert_eq!(
        exact_realized_macro_tape(&candidate, std::slice::from_ref(&first))
            .unwrap()
            .frames,
        vec![first.clone()]
    );
    assert!(exact_realized_macro_tape(&candidate, &[second]).is_err());
    assert!(exact_realized_macro_tape(&candidate, &[]).is_err());
    assert!(
        exact_realized_macro_tape(&candidate, &[first.clone(), first, InputFrame::default()])
            .is_err()
    );
}

#[test]
fn goal_seek_reserves_room_for_reactive_redirection() {
    assert_eq!(goal_tactic_maximum_ticks(160).unwrap(), 40);
    assert_eq!(goal_tactic_maximum_ticks(3).unwrap(), 1);
    assert_eq!(goal_tactic_maximum_ticks(1_000).unwrap(), 40);
    assert!(goal_tactic_maximum_ticks(0).is_err());
    assert_eq!(goal_route_sequence_maximum_ticks(160).unwrap(), 40);
    assert_eq!(goal_route_sequence_maximum_ticks(1_000).unwrap(), 40);
    assert!(goal_route_sequence_maximum_ticks(0).is_err());
}

#[test]
fn live_parameterized_policy_rejects_authored_route_actions() {
    let generic = propose_parameterized_tactics(ParameterizedTacticProposalContext {
        seed: 11,
        decision_index: 3,
        state_sha256: Digest([7; 32]),
        player_position: [0.0, 0.0, 0.0],
        camera_yaw_radians: Some(0.0),
        goal_coordinate: [100.0, 0.0, -100.0],
        maximum_ticks: 40,
        feedback: None,
    })
    .unwrap();
    validate_parameterized_policy_catalog(&generic.catalog).unwrap();

    let authored =
        dusklight_learning::default_tactic_catalog::goal_conditioned_route_tactic_catalog(
            &[[100.0, 0.0, -100.0]],
            &[vec![[0.0, 0.0, 0.0], [100.0, 0.0, -100.0]]],
            40,
            40,
        )
        .unwrap();
    assert!(
        validate_parameterized_policy_catalog(&authored)
            .unwrap_err()
            .to_string()
            .contains("non-atomic authored action")
    );
}

#[test]
fn promoted_recorded_tactics_join_without_removing_primitive_actions() {
    let generic = propose_parameterized_tactics(ParameterizedTacticProposalContext {
        seed: 11,
        decision_index: 3,
        state_sha256: Digest([7; 32]),
        player_position: [0.0, 0.0, 0.0],
        camera_yaw_radians: Some(0.0),
        goal_coordinate: [100.0, 0.0, -100.0],
        maximum_ticks: 40,
        feedback: None,
    })
    .unwrap();
    let tape = InputTape {
        frames: vec![
            InputFrame {
                owned_ports: 1,
                ..InputFrame::default()
            };
            4
        ],
        ..InputTape::default()
    };
    let component = TacticMacroComponent::from_catalog_entry(
        &TacticCatalogEntry::new(
            "family/primitive",
            TacticAssetSource::RecordedTape(tape.clone()),
        )
        .unwrap(),
    )
    .unwrap();
    let candidate = replay_macro_candidate(
        tape,
        vec![component],
        vec![
            MacroSourceProvenance {
                seed: 11,
                frontier_state_sha256: Digest([1; 32]),
                transition_sha256s: vec![Digest([2; 32])],
                entry: MacroEntryObservation {
                    stage: "F_SP103".into(),
                    room: 1,
                    player_procedure: Some(3),
                    player_contacts: Some(1),
                    goal_distance_f32_bits: 100.0_f32.to_bits(),
                },
            },
            MacroSourceProvenance {
                seed: 22,
                frontier_state_sha256: Digest([3; 32]),
                transition_sha256s: vec![Digest([4; 32])],
                entry: MacroEntryObservation {
                    stage: "F_SP103".into(),
                    room: 1,
                    player_procedure: Some(3),
                    player_contacts: Some(1),
                    goal_distance_f32_bits: 90.0_f32.to_bits(),
                },
            },
        ],
    )
    .unwrap();
    let mut entries = generic.catalog.entries().to_vec();
    entries.push(candidate.catalog_entry().unwrap());
    let combined = TacticAssetCatalog::new(entries).unwrap();

    validate_parameterized_policy_catalog(&combined).unwrap();
    assert!(
        combined
            .entries()
            .iter()
            .any(|entry| entry.option_id().starts_with("promoted/"))
    );
    assert!(
        combined
            .entries()
            .iter()
            .any(|entry| entry.option_id().starts_with("family/"))
    );
    let shard = NativeEpisodeShard::decode(include_bytes!(
        "../../../../../../tests/fixtures/automation/native_episode_v28.dseps"
    ))
    .unwrap();
    let snapshot = FactSnapshot::from_native_learning(
        &shard.episodes[0].steps[0].pre_input,
        &[],
        None,
        Vec::new(),
    )
    .unwrap();
    let target = snapshot.player.position_f32_bits.map(f32::from_bits);
    let encoder = GoalConditionedTacticFeatureEncoder::new(target).unwrap();
    let imported = ImportedPromotedTactic {
        entry: candidate.catalog_entry().unwrap(),
        condition: TacticMacroEntryCondition {
            cells: vec![TacticMacroEntryConditionCell {
                stage: snapshot.world.stage.clone(),
                room: snapshot.world.room,
                player_procedure: snapshot.player.procedure,
                player_contacts: snapshot.player.contacts,
                minimum_goal_distance: 0.0,
                maximum_goal_distance: 0.0,
            }],
        },
    };
    let primitive_proposals =
        parameterized_catalog_for_state(11, 3, &snapshot, &encoder, 40, None, Digest([8; 32]))
            .unwrap();
    let primitive_descriptors = primitive_proposals
        .catalog
        .option_descriptors()
        .map(|descriptor| (descriptor.option_id.clone(), descriptor.clone()))
        .collect::<BTreeMap<_, _>>();
    let proposals = parameterized_catalog_for_state_with_promoted(
        11,
        3,
        &snapshot,
        &encoder,
        40,
        None,
        Digest([8; 32]),
        std::slice::from_ref(&imported),
    )
    .unwrap();
    let combined_descriptors = proposals
        .catalog
        .option_descriptors()
        .map(|descriptor| (descriptor.option_id.clone(), descriptor.clone()))
        .collect::<BTreeMap<_, _>>();
    assert!(primitive_descriptors.iter().all(|(option_id, descriptor)| {
        combined_descriptors.get(option_id) == Some(descriptor)
    }));
    let promoted_descriptors = combined_descriptors
        .iter()
        .filter(|(option_id, _)| !primitive_descriptors.contains_key(*option_id))
        .collect::<Vec<_>>();
    assert_eq!(promoted_descriptors.len(), 1);
    assert!(promoted_descriptors[0].1.option_id.starts_with("promoted/"));
    assert!(
        proposals
            .catalog
            .entries()
            .iter()
            .any(|entry| entry.option_id().starts_with("promoted/"))
    );
    let mut wrong_room = snapshot.clone();
    wrong_room.world.room = wrong_room.world.room.saturating_add(1);
    let wrong_room_primitive_descriptors =
        parameterized_catalog_for_state(11, 3, &wrong_room, &encoder, 40, None, Digest([8; 32]))
            .unwrap()
            .catalog
            .option_descriptors()
            .map(|descriptor| (descriptor.option_id.clone(), descriptor.clone()))
            .collect::<BTreeMap<_, _>>();
    let proposals = parameterized_catalog_for_state_with_promoted(
        11,
        3,
        &wrong_room,
        &encoder,
        40,
        None,
        Digest([8; 32]),
        std::slice::from_ref(&imported),
    )
    .unwrap();
    assert_eq!(
        proposals
            .catalog
            .option_descriptors()
            .map(|descriptor| (descriptor.option_id.clone(), descriptor.clone()))
            .collect::<BTreeMap<_, _>>(),
        wrong_room_primitive_descriptors
    );
    assert!(
        proposals
            .catalog
            .entries()
            .iter()
            .all(|entry| !entry.option_id().starts_with("promoted/"))
    );
    let learner = LearnerState::build(snapshot, &FactRegistry::canonical(), &combined, &[], |_| {
        true
    })
    .unwrap();
    assert!(
        learner
            .applicable_descriptors()
            .any(|descriptor| descriptor.option_id.starts_with("promoted/"))
    );
    assert!(
        learner
            .applicable_descriptors()
            .any(|descriptor| descriptor.option_id.starts_with("family/"))
    );
}

#[test]
fn goal_corridor_is_a_symmetric_start_and_goal_derived_action_basis() {
    let source = [0.0, 10.0, 0.0];
    let goal = [1000.0, 20.0, 0.0];
    let (targets, route_sequences) = goal_corridor_targets(source, goal).unwrap();

    assert_eq!(targets.len(), 20);
    assert_eq!(targets[0], goal);
    assert!(targets.contains(&[250.0, 12.5, -768.0]));
    assert!(targets.contains(&[250.0, 12.5, 768.0]));
    assert!(targets.contains(&[500.0, 15.0, 0.0]));
    assert_eq!(
        targets
            .iter()
            .map(|target| target.map(f32::to_bits))
            .collect::<BTreeSet<_>>()
            .len(),
        targets.len()
    );
    assert_eq!(route_sequences.len(), 5);
    assert!(route_sequences.iter().all(|route| route.len() == 4));
    assert_eq!(route_sequences[0][0], [250.0, 12.5, -768.0]);
    assert_eq!(route_sequences[2][1], [500.0, 15.0, 0.0]);
    assert!(
        route_sequences
            .iter()
            .all(|route| route.last() == Some(&goal))
    );
    assert!(goal_corridor_targets(source, source).is_err());
}

#[test]
fn navigable_surface_route_follows_ground_adjacency_with_bounded_sampling() {
    let nodes = (0..=4)
        .map(|index| NavigableSurfaceNode {
            collision_id: format!("surface-{index}"),
            coordinate: [index as f32 * 100.0, 10.0, 0.0],
        })
        .collect::<Vec<_>>();
    let edges = (0..4)
        .map(|index| NavigableSurfaceEdge {
            left_collision_id: format!("surface-{index}"),
            right_collision_id: format!("surface-{}", index + 1),
            shared_edge: [
                [index as f32 * 100.0 + 50.0, 10.0, -50.0],
                [index as f32 * 100.0 + 50.0, 10.0, 50.0],
            ],
        })
        .collect::<Vec<_>>();

    let routes =
        shortest_navigable_surface_routes(&nodes, &edges, [0.0, 10.0, 0.0], [400.0, 10.0, 0.0])
            .unwrap()
            .expect("connected ground surfaces must produce routes");

    assert_eq!(routes, vec![vec![[400.0, 10.0, 0.0]]]);
    assert!(
        shortest_navigable_surface_routes(
            &nodes,
            &edges[..2],
            [0.0, 10.0, 0.0],
            [400.0, 10.0, 0.0],
        )
        .unwrap()
        .is_none()
    );
}

#[test]
fn navigable_surface_route_minimizes_travel_distance_not_triangle_count() {
    let nodes = vec![
        NavigableSurfaceNode {
            collision_id: "source".into(),
            coordinate: [0.0, 10.0, 0.0],
        },
        NavigableSurfaceNode {
            collision_id: "detour".into(),
            coordinate: [200.0, 10.0, 1_000.0],
        },
        NavigableSurfaceNode {
            collision_id: "straight-a".into(),
            coordinate: [100.0, 10.0, 0.0],
        },
        NavigableSurfaceNode {
            collision_id: "straight-b".into(),
            coordinate: [300.0, 10.0, 0.0],
        },
        NavigableSurfaceNode {
            collision_id: "goal".into(),
            coordinate: [400.0, 10.0, 0.0],
        },
    ];
    let edge = |left: &str, right: &str, x: f32, z: f32| NavigableSurfaceEdge {
        left_collision_id: left.into(),
        right_collision_id: right.into(),
        shared_edge: [[x, 10.0, z - 50.0], [x, 10.0, z + 50.0]],
    };
    let edges = vec![
        edge("source", "detour", 100.0, 500.0),
        edge("detour", "goal", 300.0, 500.0),
        edge("source", "straight-a", 50.0, 0.0),
        edge("straight-a", "straight-b", 200.0, 0.0),
        edge("straight-b", "goal", 350.0, 0.0),
    ];

    let routes =
        shortest_navigable_surface_routes(&nodes, &edges, nodes[0].coordinate, nodes[4].coordinate)
            .unwrap()
            .expect("both corridors connect the source and goal");

    assert_eq!(routes[0], vec![nodes[4].coordinate]);
}

#[test]
fn load_trigger_target_is_inside_the_nearest_real_surface() {
    let near = [[0.0, 10.0, 0.0], [300.0, 10.0, 0.0], [0.0, 10.0, 300.0]];
    let far = [
        [1_000.0, 20.0, 0.0],
        [1_300.0, 20.0, 0.0],
        [1_000.0, 20.0, 300.0],
    ];

    let target = nearest_interior_load_trigger_target([-100.0, 0.0, -100.0], &[far, near], 60.0)
        .expect("a reconstructed trigger surface produces a target");

    let expected = 60.0 / 2.0_f32.sqrt();
    assert!((target[0] - expected).abs() < 0.001);
    assert_eq!(target[1], 10.0);
    assert!((target[2] - expected).abs() < 0.001);
    assert!(nearest_interior_load_trigger_target([f32::NAN, 0.0, 0.0], &[near], 60.0).is_none());
}

#[test]
fn planar_trigger_projection_handles_points_inside_and_outside() {
    let triangle = [[0.0, 10.0, 0.0], [100.0, 20.0, 0.0], [0.0, 30.0, 100.0]];

    assert_eq!(
        closest_planar_point_on_triangle([25.0, 0.0, 25.0], triangle),
        Some([25.0, 17.5, 25.0])
    );
    assert_eq!(
        closest_planar_point_on_triangle([-20.0, 0.0, -30.0], triangle),
        Some([0.0, 10.0, 0.0])
    );
}

#[test]
fn surface_funnel_string_pulls_portals_instead_of_chasing_their_centers() {
    let nodes = vec![
        NavigableSurfaceNode {
            collision_id: "a".into(),
            coordinate: [0.0, 10.0, 0.0],
        },
        NavigableSurfaceNode {
            collision_id: "b".into(),
            coordinate: [100.0, 10.0, 0.0],
        },
        NavigableSurfaceNode {
            collision_id: "c".into(),
            coordinate: [200.0, 10.0, 100.0],
        },
        NavigableSurfaceNode {
            collision_id: "d".into(),
            coordinate: [300.0, 10.0, 0.0],
        },
    ];
    let edges = vec![
        NavigableSurfaceEdge {
            left_collision_id: "a".into(),
            right_collision_id: "b".into(),
            shared_edge: [[100.0, 10.0, -100.0], [100.0, 10.0, 100.0]],
        },
        NavigableSurfaceEdge {
            left_collision_id: "b".into(),
            right_collision_id: "c".into(),
            shared_edge: [[200.0, 10.0, 50.0], [200.0, 10.0, 150.0]],
        },
        NavigableSurfaceEdge {
            left_collision_id: "c".into(),
            right_collision_id: "d".into(),
            shared_edge: [[300.0, 10.0, -100.0], [300.0, 10.0, 100.0]],
        },
    ];

    let route = funnel_surface_route(
        &nodes,
        &[0, 1, 2, 3],
        &edges,
        &[0, 1, 2],
        [0.0, 10.0, 0.0],
        [400.0, 10.0, 0.0],
        0.0,
    )
    .unwrap();

    assert_eq!(route, vec![[200.0, 10.0, 50.0], [400.0, 10.0, 0.0]]);
}

#[test]
fn surface_route_simplification_spends_targets_on_actual_turns() {
    let path = [
        [0.0, 10.0, 0.0],
        [100.0, 10.0, 0.0],
        [200.0, 10.0, 100.0],
        [300.0, 10.0, 100.0],
        [400.0, 10.0, 0.0],
    ];

    let simplified = simplify_planar_surface_route(&path, 3);

    assert_eq!(simplified.len(), 3);
    assert_eq!(simplified[0], path[0]);
    assert_eq!(simplified[2], path[4]);
    assert_eq!(simplified[1][2], 100.0);
}

#[test]
fn surface_route_precedes_but_does_not_replace_generic_corridor_actions() {
    let source = [0.0, 10.0, 0.0];
    let goal = [1000.0, 20.0, 0.0];
    let surface = vec![
        [200.0, 11.0, 50.0],
        [500.0, 15.0, 75.0],
        [800.0, 18.0, 25.0],
        goal,
    ];
    let (targets, routes, ids) = fallback_goal_routes(
        source,
        goal,
        Some(vec![("surface-graph:test".into(), surface.clone())]),
    )
    .unwrap();

    assert_eq!(routes.len(), 5);
    assert_eq!(routes[0], surface);
    assert_eq!(ids, vec!["surface-graph:test"]);
    assert!(targets.contains(&[500.0, 15.0, 75.0]));
    assert!(
        routes
            .iter()
            .skip(1)
            .all(|route| route.last() == Some(&goal))
    );
}

#[test]
fn authored_room_paths_replace_the_synthetic_corridor_with_attached_routes() {
    use dusklight_world::world_inventory::{
        AuthoredPathPointRecord, AuthoredPathRecord, SourceKind, SourceScope,
        WORLD_INVENTORY_SCHEMA,
    };

    let source_digest = Digest([7; 32]);
    let scope = SourceScope {
        kind: SourceKind::Room,
        room: Some(1),
    };
    let path = |record_index, first_point_index, point_count| AuthoredPathRecord {
        stable_id: format!("path/{record_index}"),
        source_sha256: source_digest,
        scope,
        record_index,
        point_count,
        next_path_index: None,
        path_argument: u8::MAX,
        closed: false,
        closed_raw: 0,
        switch_no: None,
        unknown_07: 0,
        point_offset: u32::try_from(first_point_index * 16).unwrap(),
        first_point_index,
        raw_hex: "00".repeat(12),
    };
    let point = |record_index, position: [f32; 3]| AuthoredPathPointRecord {
        stable_id: format!("point/{record_index}"),
        source_sha256: source_digest,
        scope,
        record_index,
        arguments: [u8::MAX; 4],
        position: dusklight_world::world_geometry::Vec3 {
            x: position[0],
            y: position[1],
            z: position[2],
        },
        raw_hex: "00".repeat(16),
    };
    let inventory = WorldInventory {
        schema: WORLD_INVENTORY_SCHEMA.into(),
        stage: "TEST".into(),
        sources: Vec::new(),
        chunks: Vec::new(),
        placements: Vec::new(),
        player_spawns: Vec::new(),
        exits: Vec::new(),
        paths: vec![path(0, 0, 2), path(1, 2, 2), path(2, 4, 2)],
        path_points: vec![
            point(0, [100.0, 0.0, 100.0]),
            point(1, [100.0, 0.0, 900.0]),
            point(2, [300.0, 0.0, 200.0]),
            point(3, [300.0, 0.0, 700.0]),
            point(4, [450.0, 0.0, 300.0]),
            point(5, [450.0, 0.0, 600.0]),
        ],
        collisions: Vec::new(),
        load_triggers: Vec::new(),
    };
    let goal = [0.0, 0.0, 1_000.0];
    let (targets, routes, route_ids) =
        goal_route_targets([0.0, 0.0, 0.0], goal, 1, &inventory).unwrap();

    assert_eq!(routes.len(), 2);
    assert_eq!(route_ids.len(), routes.len());
    assert_eq!(routes[0][0], [100.0, 0.0, 100.0]);
    assert_eq!(routes[0][1], [100.0, 0.0, 900.0]);
    assert_eq!(routes[0].last(), Some(&goal));
    assert!(route_ids[0].contains("path/0:forward"));
    assert_eq!(targets[0], goal);
    assert!(targets.contains(&[100.0, 0.0, 100.0]));
}

#[test]
fn real_f_sp104_authored_main_path_is_the_bootstrap_route_when_disc_is_present() {
    let stage_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../..")
        .join("orig/GZ2E01/files/res/Stage/F_SP104");
    if !stage_dir.is_dir() {
        eprintln!("skipping F_SP104 route-basis golden: original disc data is absent");
        return;
    }
    let inventory = WorldInventory::build(&stage_dir, "F_SP104").unwrap();
    let source = [150.21315, 306.54245, -2785.0728];
    let goal = [-430.95392, 241.77234, -21165.0];
    let (_, routes, route_ids) = goal_route_targets(source, goal, 1, &inventory).unwrap();

    assert_eq!(routes.len(), 1);
    assert!(route_ids[0].contains("/chunk/RPAT/record/14:forward"));
    assert_eq!(routes[0][0], [300.0, 270.81253, -3950.0]);
    assert_eq!(routes[0][7], [-441.90887, 314.0304, -19270.963]);
    assert_eq!(routes[0].last(), Some(&goal));
}

#[test]
fn route_attempts_are_append_only_across_resume_launches() {
    let directory = std::env::temp_dir().join(format!(
        "dusklight-tactic-route-attempts-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).unwrap();

    let first = reserve_attempt_root(&directory).unwrap();
    let second = reserve_attempt_root(&directory).unwrap();

    assert_eq!(first.file_name().unwrap(), "attempt-0000");
    assert_eq!(second.file_name().unwrap(), "attempt-0001");
    assert!(first.is_dir());
    assert!(second.is_dir());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn checkpoint_cache_capacity_is_derived_from_the_sealed_aggregate_memory_budget() {
    assert_eq!(
        tactic_checkpoint_cache_capacity_per_worker(NativeTacticResourceLimit::Bounded(1_000), 4)
            .unwrap(),
        250
    );
    assert_eq!(
        tactic_checkpoint_cache_capacity_per_worker(
            NativeTacticResourceLimit::Bounded(
                (TACTIC_CHECKPOINT_CACHE_BYTES as u64).saturating_mul(8)
            ),
            4
        )
        .unwrap(),
        TACTIC_CHECKPOINT_CACHE_BYTES
    );
    assert_eq!(
        tactic_checkpoint_cache_capacity_per_worker(NativeTacticResourceLimit::Unbounded, 16)
            .unwrap(),
        TACTIC_CHECKPOINT_CACHE_BYTES
    );
    assert!(
        tactic_checkpoint_cache_capacity_per_worker(NativeTacticResourceLimit::Bounded(3), 4)
            .is_err()
    );
}

#[test]
fn checkpoint_capacity_can_reserve_a_wider_fleet_share_without_launching_it() {
    assert!(valid_worker_capacity_counts(1, 16));
    assert!(valid_worker_capacity_counts(16, 16));
    assert!(!valid_worker_capacity_counts(0, 16));
    assert!(!valid_worker_capacity_counts(2, 1));
    assert!(!valid_worker_capacity_counts(1, MAX_ROUTE_WORKERS + 1));
}
