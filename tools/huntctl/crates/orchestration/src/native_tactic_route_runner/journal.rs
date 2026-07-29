use super::*;

pub fn tactic_decision_journal_path(seed_root: &Path) -> PathBuf {
    seed_root.join(NATIVE_TACTIC_DECISION_JOURNAL_FILE)
}

pub fn has_tactic_decision_journal(seed_root: &Path) -> bool {
    tactic_decision_journal_path(seed_root).is_file()
        || tactic_decision_segments_root(seed_root).is_dir()
}

pub fn tactic_content_store_path(seed_root: &Path) -> PathBuf {
    let legacy_local = seed_root.join(NATIVE_TACTIC_CONTENT_STORE_DIRECTORY);
    if legacy_local.exists() {
        return legacy_local;
    }
    // New campaigns share immutable content objects across seed directories.
    // This turns imported replay from an O(seeds²) rewrite into references to
    // objects already durably installed by prior generations. Legacy
    // per-seed stores remain discoverable and resume without migration.
    if seed_root
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("seed-"))
        && let Some(campaign_root) = seed_root.parent()
    {
        return campaign_root.join(NATIVE_TACTIC_CONTENT_STORE_DIRECTORY);
    }
    legacy_local
}

pub fn read_tactic_decision_journal(
    seed_root: &Path,
) -> Result<Vec<NativeTacticDecisionTrace>, NativeTacticRouteRunError> {
    if !has_tactic_decision_journal(seed_root) {
        return Ok(Vec::new());
    }
    let store =
        TacticQContentStore::open(tactic_content_store_path(seed_root)).map_err(route_error)?;
    read_tactic_decision_records(seed_root)?
        .into_iter()
        .map(|record| project_tactic_decision_record(&store, record))
        .collect()
}

pub(super) fn read_tactic_decision_records(
    seed_root: &Path,
) -> Result<Vec<NativeTacticDecisionRecord>, NativeTacticRouteRunError> {
    let mut records = read_compacted_tactic_decision_records(seed_root)?;
    let path = tactic_decision_journal_path(seed_root);
    if !path.exists() {
        return Ok(records);
    }
    let metadata = fs::symlink_metadata(&path).map_err(route_error)?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(route_message(
            "tactic decision journal is not a physical file",
        ));
    }
    let bytes = fs::read(&path).map_err(route_error)?;
    let decoded = decode_tactic_decision_journal(&bytes)?;
    for record in decoded.records {
        let index = usize::try_from(record.decision_index).map_err(route_error)?;
        if index < records.len() {
            if records[index] != record {
                return Err(route_message(
                    "active tactic journal conflicts with its compacted segment",
                ));
            }
        } else if index == records.len() {
            records.push(record);
        } else {
            return Err(route_message(
                "active tactic journal is detached from its compacted segments",
            ));
        }
    }
    Ok(records)
}

pub fn project_tactic_decision_graph(
    seed_root: &Path,
) -> Result<Option<TacticCampaignGraphProjection>, NativeTacticRouteRunError> {
    if !has_tactic_decision_journal(seed_root) {
        return Ok(None);
    }
    let replay = load_tactic_journal_replay(seed_root)?;
    let first = replay
        .transitions
        .first()
        .ok_or_else(|| route_message("tactic graph has no root transition"))?;
    let last = replay
        .transitions
        .last()
        .ok_or_else(|| route_message("tactic graph has no current transition"))?;
    let root_checkpoint_sha256 = first.source_checkpoint_sha256;
    let root_state_sha256 = first.before_state_sha256;
    let current = (last.next_checkpoint_sha256, last.after_state_sha256);
    let mut archive = BehaviorArchive::default();
    for (index, (transition, route)) in replay.transitions.iter().zip(&replay.routes).enumerate() {
        archive
            .consider_tactic_endpoint(
                replay.root_checkpoint_sha256,
                transition.clone(),
                route.clone(),
                index as u64,
            )
            .map_err(route_error)?;
    }
    let retained = archive.tactic_route_checkpoints().collect::<BTreeSet<_>>();
    let mut nodes = BTreeMap::new();
    let mut edges = Vec::with_capacity(replay.records.len());
    for (record, transition) in replay.records.iter().zip(&replay.transitions) {
        for (checkpoint_sha256, state_sha256, state, terminal) in [
            (
                transition.source_checkpoint_sha256,
                transition.before_state_sha256,
                &transition.before,
                transition.before.terminal.reached == Some(true),
            ),
            (
                transition.next_checkpoint_sha256,
                transition.after_state_sha256,
                &transition.after,
                transition.value_sample.terminal,
            ),
        ] {
            let identity = (checkpoint_sha256, state_sha256);
            let node = TacticCampaignGraphProjectionNode {
                checkpoint_sha256,
                state_sha256,
                stage: state.world.stage.clone(),
                room: state.world.room,
                player_position: state.player.position_f32_bits.map(f32::from_bits),
                terminal,
                retained_frontier: retained.contains(&checkpoint_sha256),
                current: identity == current,
            };
            if nodes
                .insert(identity, node.clone())
                .is_some_and(|existing| existing != node)
            {
                return Err(route_message(
                    "tactic decision journal has conflicting graph nodes",
                ));
            }
        }
        edges.push(TacticCampaignGraphProjectionEdge {
            edge_index: record.decision_index,
            episode_group: record.episode_group,
            before_state_sha256: transition.before_state_sha256,
            after_state_sha256: transition.after_state_sha256,
            source_checkpoint_sha256: transition.source_checkpoint_sha256,
            next_checkpoint_sha256: transition.next_checkpoint_sha256,
            option_id: transition.value_sample.action.option_id.clone(),
            reward: record.reward,
            duration_ticks: transition.execution.duration.realized_ticks,
            terminal: transition.value_sample.terminal,
            start_frame: transition.execution.realized_tape_range.start_frame,
            end_frame_exclusive: transition.execution.realized_tape_range.end_frame_exclusive,
        });
    }
    let mut reachable = BTreeSet::from([(root_checkpoint_sha256, root_state_sha256)]);
    loop {
        let before = reachable.len();
        for edge in &edges {
            if reachable.contains(&(edge.source_checkpoint_sha256, edge.before_state_sha256)) {
                reachable.insert((edge.next_checkpoint_sha256, edge.after_state_sha256));
            }
        }
        if reachable.len() == before {
            break;
        }
    }
    Ok(Some(TacticCampaignGraphProjection {
        schema: "dusklight-tactic-campaign-graph-projection/v1".into(),
        root_checkpoint_sha256,
        root_state_sha256,
        root_connected: nodes.keys().all(|identity| reachable.contains(identity)),
        frontier_cells: retained.len(),
        nodes: nodes.into_values().collect(),
        edges,
    }))
}

pub fn project_tactic_decision_diagnostics(
    seed_root: &Path,
) -> Result<Option<TacticCampaignDiagnostics>, NativeTacticRouteRunError> {
    if !has_tactic_decision_journal(seed_root) {
        return Ok(None);
    }
    let replay = load_tactic_journal_replay(seed_root)?;
    let graph = project_tactic_decision_graph(seed_root)?
        .ok_or_else(|| route_message("tactic diagnostics have no graph"))?;
    let mut compositions = BTreeMap::<u64, Vec<Digest>>::new();
    let mut selected_actions = BTreeSet::new();
    for (record, transition) in replay.records.iter().zip(&replay.transitions) {
        let bytes = serde_cbor::to_vec(&transition.value_sample.action).map_err(route_error)?;
        let digest = Digest(Sha256::digest(&bytes).into());
        selected_actions.insert(digest);
        compositions
            .entry(record.episode_group)
            .or_default()
            .push(digest);
    }
    let mut composition_counts = BTreeMap::<Vec<Digest>, usize>::new();
    for composition in compositions.into_values().filter(|row| !row.is_empty()) {
        *composition_counts.entry(composition).or_default() += 1;
    }
    let episode_groups = replay
        .records
        .iter()
        .map(|record| record.episode_group)
        .collect::<Vec<_>>();
    let (logical_frontier_records, directly_restorable_native_frontiers, replay_only_frontiers) =
        replay
            .records
            .last()
            .map_or((graph.nodes.len(), 0, graph.frontier_cells), |record| {
                if record.logical_frontier_records == 0 {
                    (graph.nodes.len(), 0, graph.frontier_cells)
                } else {
                    (
                        record.logical_frontier_records,
                        record.directly_restorable_native_frontiers,
                        record.replay_only_frontiers,
                    )
                }
            });
    Ok(Some(TacticCampaignDiagnostics {
        replay_rows: replay.transitions.len(),
        frontier_cells: graph.frontier_cells,
        logical_frontier_records,
        directly_restorable_native_frontiers,
        replay_only_frontiers,
        unique_selected_actions: selected_actions.len(),
        zero_diversity_selection: replay.transitions.len() >= 2 && selected_actions.len() <= 1,
        repeated_identical_compositions: composition_counts.values().any(|count| *count > 1),
        no_progress_loop: has_no_progress_loop(&replay.transitions, &episode_groups)
            .map_err(route_error)?,
        frontier_lost_root_connectivity: !graph.root_connected,
    }))
}

pub fn materialize_tactic_decision_route(
    seed_root: &Path,
    decision_index: u64,
) -> Result<InputTape, NativeTacticRouteRunError> {
    let replay = load_tactic_journal_replay(seed_root)?;
    let target_index = usize::try_from(decision_index).map_err(route_error)?;
    let target_record = replay
        .records
        .get(target_index)
        .ok_or_else(|| route_message("tactic decision route is absent"))?;
    if target_record.decision_index != decision_index {
        return Err(route_message("tactic decision route index is detached"));
    }
    replay
        .routes
        .get(target_index)
        .cloned()
        .ok_or_else(|| route_message("tactic decision route is absent"))
}

pub(super) struct TacticJournalReplay {
    pub(super) root_checkpoint_sha256: Digest,
    pub(super) records: Vec<NativeTacticDecisionRecord>,
    pub(super) transitions: Vec<dusklight_learning::option_transition::OptionTransitionSample>,
    pub(super) routes: Vec<InputTape>,
}

pub(super) fn load_tactic_journal_replay(
    seed_root: &Path,
) -> Result<TacticJournalReplay, NativeTacticRouteRunError> {
    let records = read_tactic_decision_records(seed_root)?;
    let first_record = records
        .first()
        .ok_or_else(|| route_message("tactic decision journal is empty"))?;
    if first_record.root_checkpoint_sha256 == Digest::ZERO
        || records.iter().any(|record| {
            record.root_checkpoint_sha256 != first_record.root_checkpoint_sha256
                || record.root_tape != first_record.root_tape
        })
    {
        return Err(route_message(
            "tactic decision journal has conflicting root identities",
        ));
    }
    let store =
        TacticQContentStore::open(tactic_content_store_path(seed_root)).map_err(route_error)?;
    let root_tape = store
        .load_tape(first_record.root_tape)
        .map_err(route_error)?;
    let transitions = records
        .iter()
        .map(|record| {
            journal_transition(&store, record.transition, record.inline_transition.as_ref())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let proposal_transitions = records
        .iter()
        .zip(&transitions)
        .map(|(record, selected)| {
            if record.proposal_batch.is_empty() {
                Ok(vec![selected.clone()])
            } else {
                record
                    .proposal_batch
                    .iter()
                    .map(|proposal| {
                        journal_transition(
                            &store,
                            proposal.transition,
                            proposal.inline_transition.as_ref(),
                        )
                    })
                    .collect()
            }
        })
        .collect::<Result<Vec<Vec<_>>, NativeTacticRouteRunError>>()?;
    let source_route_tapes = records
        .iter()
        .map(|record| {
            record
                .source_route_tape
                .map(|reference| store.load_tape(reference).map_err(route_error))
                .transpose()
        })
        .collect::<Result<Vec<_>, _>>()?;
    let first_transition = transitions
        .first()
        .ok_or_else(|| route_message("tactic decision journal has no root transition"))?;
    if route_checkpoint(first_record.root_checkpoint_sha256, &root_tape).map_err(route_error)?
        != first_transition.source_checkpoint_sha256
        || root_tape.frames.len() as u64
            != first_transition.execution.realized_tape_range.start_frame
    {
        return Err(route_message(
            "tactic decision journal root tape is detached",
        ));
    }
    let root_identity = (
        first_transition.source_checkpoint_sha256,
        first_transition.before_state_sha256,
    );
    let mut parents = BTreeMap::<(Digest, Digest), (usize, usize)>::new();
    for (decision_index, proposals) in proposal_transitions.iter().enumerate() {
        for (proposal_index, transition) in proposals.iter().enumerate() {
            parents
                .entry((
                    transition.next_checkpoint_sha256,
                    transition.after_state_sha256,
                ))
                .or_insert((decision_index, proposal_index));
        }
    }
    let routes = transitions
        .iter()
        .enumerate()
        .map(|(index, transition)| {
            materialize_journal_route(
                index,
                transition,
                &root_tape,
                first_record.root_checkpoint_sha256,
                root_identity,
                &parents,
                &proposal_transitions,
                source_route_tapes[index].as_ref(),
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    for ((record, transition), route) in records.iter().zip(&transitions).zip(&routes) {
        transition
            .execution
            .validate_against_tape(route)
            .map_err(route_error)?;
        if record.terminal != transition.value_sample.terminal
            || record.reward_components.duration_ticks
                != transition.execution.duration.realized_ticks
            || route_checkpoint(first_record.root_checkpoint_sha256, route).map_err(route_error)?
                != transition.next_checkpoint_sha256
            || route.frames.len() as u64
                != transition.execution.realized_tape_range.end_frame_exclusive
        {
            return Err(route_message(
                "tactic decision journal transition route is detached",
            ));
        }
    }
    Ok(TacticJournalReplay {
        root_checkpoint_sha256: first_record.root_checkpoint_sha256,
        records,
        transitions,
        routes,
    })
}

pub(super) fn materialize_journal_route(
    target_decision_index: usize,
    target: &dusklight_learning::option_transition::OptionTransitionSample,
    root_tape: &InputTape,
    root_checkpoint_sha256: Digest,
    root_identity: (Digest, Digest),
    parents: &BTreeMap<(Digest, Digest), (usize, usize)>,
    proposal_transitions: &[Vec<dusklight_learning::option_transition::OptionTransitionSample>],
    source_route_tape: Option<&InputTape>,
) -> Result<InputTape, NativeTacticRouteRunError> {
    if let Some(source_route_tape) = source_route_tape {
        source_route_tape.validate().map_err(route_error)?;
        if source_route_tape.frames.len() as u64 != target.execution.realized_tape_range.start_frame
            || route_checkpoint(root_checkpoint_sha256, source_route_tape).map_err(route_error)?
                != target.source_checkpoint_sha256
        {
            return Err(route_message(
                "tactic decision source-route anchor is detached",
            ));
        }
        let mut route = source_route_tape.clone();
        route
            .frames
            .extend(target.execution.emitted_raw_actions.clone());
        route.validate().map_err(route_error)?;
        return Ok(route);
    }
    let mut cursor = (target.source_checkpoint_sha256, target.before_state_sha256);
    let mut fragments = vec![target.execution.emitted_raw_actions.clone()];
    while cursor != root_identity {
        let (parent_decision_index, parent_proposal_index) = *parents
            .get(&cursor)
            .ok_or_else(|| route_message("tactic decision route is detached from its root"))?;
        if parent_decision_index >= target_decision_index {
            return Err(route_message(
                "tactic decision route parent is not journaled before its child",
            ));
        }
        let parent = proposal_transitions
            .get(parent_decision_index)
            .and_then(|proposals| proposals.get(parent_proposal_index))
            .ok_or_else(|| route_message("tactic decision route parent is absent"))?;
        fragments.push(parent.execution.emitted_raw_actions.clone());
        cursor = (parent.source_checkpoint_sha256, parent.before_state_sha256);
        if fragments.len() > proposal_transitions.len() {
            return Err(route_message("tactic decision route contains a cycle"));
        }
    }
    let mut route = root_tape.clone();
    for fragment in fragments.into_iter().rev() {
        route.frames.extend(fragment);
    }
    route.validate().map_err(route_error)?;
    Ok(route)
}

pub(super) fn decision_record(
    trace: &NativeTacticDecisionTrace,
    episode_group: u64,
    root_checkpoint_sha256: Digest,
    root_tape: StoredContentRef,
    source_route_tape: Option<StoredContentRef>,
    transition: Option<StoredContentRef>,
    inline_transition: Option<OptionTransitionSample>,
    proposal_batch: Vec<NativeTacticProposalRecord>,
) -> NativeTacticDecisionRecord {
    NativeTacticDecisionRecord {
        execution_plan_sha256: trace.execution_plan_sha256,
        decision_index: trace.decision_index,
        cumulative_wall_micros: trace.cumulative_wall_micros,
        learner_snapshot_sha256: trace.learner_snapshot_sha256,
        replay_rows_at_decision: trace.replay_rows_at_decision,
        replay_generation: trace.replay_generation,
        lane_index: trace.lane_index,
        lane_role: trace.lane_role,
        acquisition_rank: trace.acquisition_rank,
        frontier_identity: trace.frontier_identity,
        checkpoint_owner_worker_slot: trace.checkpoint_owner_worker_slot,
        proposal_worker_slots: trace.proposal_worker_slots.clone(),
        restore_source: trace.restore_source,
        result_admission_schema: trace.result_admission_schema.clone(),
        episode: trace.episode,
        episode_group,
        source_route_ticks: trace.source_route_ticks,
        route_suffix_ticks: trace.route_suffix_ticks,
        selection_reason: trace.selection_reason,
        selected_q: trace.selected_q,
        best_q: trace.best_q,
        reward: trace.reward,
        reward_components: trace.reward_components.clone(),
        goal_distance_before: trace.goal_distance_before,
        goal_distance_after: trace.goal_distance_after,
        terminal: trace.terminal,
        newly_admitted_training_rows: trace.newly_admitted_training_rows,
        duplicate_training_transitions: trace.duplicate_training_transitions,
        training_replay_rows: trace.training_replay_rows,
        scheduler_decision: trace.scheduler_decision.clone(),
        branch_acquisition: trace.branch_acquisition.clone(),
        frontier_cells: trace.frontier_cells,
        logical_frontier_records: trace.logical_frontier_records,
        directly_restorable_native_frontiers: trace.directly_restorable_native_frontiers,
        replay_only_frontiers: trace.replay_only_frontiers,
        visited_states: trace.visited_states,
        completed_executable_graph_expansions: trace.completed_executable_graph_expansions,
        applicable_tactics: trace.applicable_tactics.clone(),
        root_checkpoint_sha256,
        root_tape,
        source_route_tape,
        transition,
        inline_transition,
        proposal_feedback: trace.proposal_feedback,
        proposal_batch,
    }
}

pub(super) fn journal_transition(
    store: &TacticQContentStore,
    reference: Option<StoredContentRef>,
    inline: Option<&OptionTransitionSample>,
) -> Result<OptionTransitionSample, NativeTacticRouteRunError> {
    match (reference, inline) {
        (Some(reference), None) => store.load_option_transition(reference).map_err(route_error),
        (None, Some(transition)) => {
            transition.validate().map_err(route_error)?;
            Ok(transition.clone())
        }
        _ => Err(route_message(
            "tactic journal transition must have exactly one storage representation",
        )),
    }
}

pub(super) fn journal_transition_sha256(
    reference: Option<StoredContentRef>,
    inline: Option<&OptionTransitionSample>,
) -> Result<Digest, NativeTacticRouteRunError> {
    match (reference, inline) {
        (Some(reference), None) => Ok(reference.sha256),
        (None, Some(transition)) => transition.replay_identity_sha256().map_err(route_error),
        _ => Err(route_message(
            "tactic journal transition must have exactly one storage representation",
        )),
    }
}

pub(super) fn project_tactic_decision_record(
    store: &TacticQContentStore,
    record: NativeTacticDecisionRecord,
) -> Result<NativeTacticDecisionTrace, NativeTacticRouteRunError> {
    let transition =
        journal_transition(store, record.transition, record.inline_transition.as_ref())?;
    if transition.execution.duration.realized_ticks != record.reward_components.duration_ticks
        || transition.value_sample.action.option_id.is_empty()
        || transition.before.content_sha256().map_err(route_error)? == Digest::ZERO
        || transition.after.content_sha256().map_err(route_error)? == Digest::ZERO
    {
        return Err(route_message(
            "tactic decision journal references are detached",
        ));
    }
    if record.logical_frontier_records != 0
        && (record.logical_frontier_records != record.frontier_cells.saturating_add(1)
            || record
                .directly_restorable_native_frontiers
                .saturating_add(record.replay_only_frontiers)
                != record.frontier_cells)
    {
        return Err(route_message(
            "tactic frontier availability accounting is detached",
        ));
    }
    let proposal_batch = record
        .proposal_batch
        .iter()
        .map(|proposal| {
            let candidate = journal_transition(
                store,
                proposal.transition,
                proposal.inline_transition.as_ref(),
            )?;
            if candidate.value_sample.action.option_id != proposal.trace.option_id
                || candidate.execution.duration.realized_ticks != proposal.trace.realized_ticks
                || (proposal.trace.emitted_tape_sha256 != Digest::ZERO
                    && candidate.value_sample.realized_tape_sha256
                        != proposal.trace.emitted_tape_sha256)
                || candidate.value_sample.terminal != proposal.trace.terminal
                || candidate.after_state_sha256 != proposal.trace.after_snapshot_sha256
                || candidate.value_sample.reward.to_bits() != proposal.trace.reward.to_bits()
            {
                return Err(route_message(
                    "tactic proposal journal reference is detached",
                ));
            }
            Ok(proposal.trace.clone())
        })
        .collect::<Result<Vec<_>, NativeTacticRouteRunError>>()?;
    if !proposal_batch.is_empty()
        && (proposal_batch
            .iter()
            .filter(|proposal| proposal.retained)
            .count()
            != 1
            || !record.proposal_batch.iter().any(|proposal| {
                proposal.trace.retained
                    && journal_transition_sha256(
                        proposal.transition,
                        proposal.inline_transition.as_ref(),
                    )
                    .ok()
                        == journal_transition_sha256(
                            record.transition,
                            record.inline_transition.as_ref(),
                        )
                        .ok()
            }))
    {
        return Err(route_message(
            "tactic proposal journal has no unique retained transition",
        ));
    }
    if record.execution_plan_sha256 != Digest::ZERO
        && proposal_batch
            .iter()
            .any(|proposal| proposal.execution_plan_sha256 != record.execution_plan_sha256)
    {
        return Err(route_message(
            "tactic proposal journal is detached from its execution plan",
        ));
    }
    if !record.proposal_worker_slots.is_empty()
        && (record.proposal_worker_slots.len() != proposal_batch.len()
            || (record.restore_source == Some(NativeTacticRestoreSource::ProcessLocalCheckpoint)
                && record.proposal_worker_slots.first().copied()
                    != record.checkpoint_owner_worker_slot))
    {
        return Err(route_message(
            "tactic proposal journal worker locality is inconsistent",
        ));
    }
    if let Some(scheduler_decision) = &record.scheduler_decision {
        scheduler_decision.validate().map_err(route_error)?;
        if scheduler_decision.learner_model_sha256 != record.learner_snapshot_sha256 {
            return Err(route_message(
                "tactic scheduler decision is detached from its learner model",
            ));
        }
    }
    Ok(NativeTacticDecisionTrace {
        execution_plan_sha256: record.execution_plan_sha256,
        decision_index: record.decision_index,
        cumulative_wall_micros: record.cumulative_wall_micros,
        learner_snapshot_sha256: record.learner_snapshot_sha256,
        replay_rows_at_decision: record.replay_rows_at_decision,
        replay_generation: record.replay_generation,
        lane_index: record.lane_index,
        lane_role: record.lane_role,
        acquisition_rank: record.acquisition_rank,
        frontier_identity: record.frontier_identity,
        checkpoint_owner_worker_slot: record.checkpoint_owner_worker_slot,
        proposal_worker_slots: record.proposal_worker_slots,
        restore_source: record.restore_source,
        result_admission_schema: record.result_admission_schema,
        episode: record.episode,
        source_route_ticks: record.source_route_ticks,
        route_suffix_ticks: record.route_suffix_ticks,
        selected_option_id: transition.value_sample.action.option_id.clone(),
        selection_reason: record.selection_reason,
        selected_q: record.selected_q,
        best_q: record.best_q,
        reward: record.reward,
        reward_components: record.reward_components,
        goal_distance_before: record.goal_distance_before,
        goal_distance_after: record.goal_distance_after,
        terminal: record.terminal,
        newly_admitted_training_rows: record.newly_admitted_training_rows,
        duplicate_training_transitions: record.duplicate_training_transitions,
        training_replay_rows: record.training_replay_rows,
        scheduler_decision: record.scheduler_decision,
        branch_acquisition: record.branch_acquisition,
        frontier_cells: record.frontier_cells,
        logical_frontier_records: record.logical_frontier_records,
        directly_restorable_native_frontiers: record.directly_restorable_native_frontiers,
        replay_only_frontiers: record.replay_only_frontiers,
        visited_states: record.visited_states,
        completed_executable_graph_expansions: record.completed_executable_graph_expansions,
        before: tactic_state_trace(&transition.before)?,
        after: tactic_state_trace(&transition.after)?,
        measurements: Vec::new(),
        applicable_tactics: record.applicable_tactics,
        proposal_feedback: record.proposal_feedback,
        proposal_batch,
    })
}

pub(super) fn append_tactic_decision_record(
    seed_root: &Path,
    decision: &NativeTacticDecisionRecord,
) -> Result<(), NativeTacticRouteRunError> {
    fs::create_dir_all(seed_root).map_err(route_error)?;
    let path = tactic_decision_journal_path(seed_root);
    ensure_tactic_decision_journal(&path)?;
    let compacted_count = compacted_tactic_decision_count(seed_root)?;
    let bytes = fs::read(&path).map_err(route_error)?;
    let decoded = decode_tactic_decision_journal(&bytes)?;
    let active_count = decoded
        .records
        .iter()
        .filter(|record| record.decision_index >= compacted_count)
        .count() as u64;
    if compacted_count.saturating_add(active_count) != decision.decision_index {
        return Err(route_message(
            "tactic decision journal append index is detached",
        ));
    }
    if decoded.valid_bytes != bytes.len() {
        OpenOptions::new()
            .write(true)
            .open(&path)
            .and_then(|file| file.set_len(decoded.valid_bytes as u64))
            .map_err(route_error)?;
    }
    let record = encode_tactic_decision_record(decision)?;
    let mut file = OpenOptions::new()
        .append(true)
        .open(&path)
        .map_err(route_error)?;
    file.write_all(&record)
        .and_then(|_| file.sync_data())
        .map_err(route_error)?;
    drop(file);
    if active_count.saturating_add(1) >= NATIVE_TACTIC_DECISION_COMPACTION_RECORDS {
        compact_tactic_decision_journal(seed_root)?;
    }
    Ok(())
}

pub(super) fn ensure_tactic_decision_journal(path: &Path) -> Result<(), NativeTacticRouteRunError> {
    if path.exists() {
        return Ok(());
    }
    let parent = path
        .parent()
        .ok_or_else(|| route_message("tactic decision journal has no parent"))?;
    let partial = parent.join(format!(
        ".{NATIVE_TACTIC_DECISION_JOURNAL_FILE}.{}.partial",
        std::process::id()
    ));
    if partial.exists() {
        fs::remove_file(&partial).map_err(route_error)?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&partial)
        .map_err(route_error)?;
    file.write_all(NATIVE_TACTIC_DECISION_JOURNAL_MAGIC)
        .and_then(|_| file.write_all(&NATIVE_TACTIC_DECISION_JOURNAL_VERSION.to_le_bytes()))
        .and_then(|_| file.write_all(&0_u16.to_le_bytes()))
        .and_then(|_| file.sync_all())
        .map_err(route_error)?;
    drop(file);
    match fs::rename(&partial, path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            fs::remove_file(&partial).map_err(route_error)
        }
        Err(error) => Err(route_error(error)),
    }
}

pub(super) struct DecodedTacticDecisionJournal {
    records: Vec<NativeTacticDecisionRecord>,
    valid_bytes: usize,
}

#[derive(Clone, Copy)]
pub(super) struct TacticDecisionSegmentHeader {
    start_index: u64,
    record_count: u64,
    raw_len: usize,
    raw_sha256: [u8; 32],
}

pub(super) fn tactic_decision_segments_root(seed_root: &Path) -> PathBuf {
    seed_root.join(NATIVE_TACTIC_DECISION_SEGMENTS_DIRECTORY)
}

pub(super) fn tactic_decision_segment_paths(
    seed_root: &Path,
) -> Result<Vec<PathBuf>, NativeTacticRouteRunError> {
    let root = tactic_decision_segments_root(seed_root);
    if !root.exists() {
        return Ok(Vec::new());
    }
    let metadata = fs::symlink_metadata(&root).map_err(route_error)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(route_message(
            "tactic decision segment root is not a physical directory",
        ));
    }
    let mut paths = fs::read_dir(&root)
        .map_err(route_error)?
        .map(|entry| entry.map(|entry| entry.path()).map_err(route_error))
        .collect::<Result<Vec<_>, _>>()?;
    paths.retain(|path| path.extension().and_then(|value| value.to_str()) == Some("dtqz"));
    if paths.len() > MAXIMUM_TACTIC_DECISION_SEGMENTS {
        return Err(route_message(
            "tactic decision journal exceeds its segment bound",
        ));
    }
    paths.sort();
    Ok(paths)
}

pub(super) fn read_tactic_decision_segment_header(
    path: &Path,
) -> Result<TacticDecisionSegmentHeader, NativeTacticRouteRunError> {
    let metadata = fs::symlink_metadata(path).map_err(route_error)?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() < NATIVE_TACTIC_DECISION_SEGMENT_HEADER_SIZE as u64
        || metadata.len()
            > (NATIVE_TACTIC_DECISION_SEGMENT_HEADER_SIZE
                + MAXIMUM_TACTIC_DECISION_SEGMENT_BYTES
                + 1024 * 1024) as u64
    {
        return Err(route_message(
            "tactic decision segment is not a physical file",
        ));
    }
    let mut bytes = [0_u8; NATIVE_TACTIC_DECISION_SEGMENT_HEADER_SIZE];
    OpenOptions::new()
        .read(true)
        .open(path)
        .and_then(|mut file| file.read_exact(&mut bytes))
        .map_err(route_error)?;
    decode_tactic_decision_segment_header(&bytes)
}

pub(super) fn decode_tactic_decision_segment_header(
    bytes: &[u8],
) -> Result<TacticDecisionSegmentHeader, NativeTacticRouteRunError> {
    if bytes.len() < NATIVE_TACTIC_DECISION_SEGMENT_HEADER_SIZE
        || &bytes[..8] != NATIVE_TACTIC_DECISION_SEGMENT_MAGIC
        || u16::from_le_bytes(bytes[8..10].try_into().expect("fixed slice"))
            != NATIVE_TACTIC_DECISION_SEGMENT_VERSION
        || u16::from_le_bytes(bytes[10..12].try_into().expect("fixed slice")) != 0
    {
        return Err(route_message("tactic decision segment header is invalid"));
    }
    let start_index = u64::from_le_bytes(bytes[12..20].try_into().expect("fixed slice"));
    let record_count = u64::from_le_bytes(bytes[20..28].try_into().expect("fixed slice"));
    let raw_len_u64 = u64::from_le_bytes(bytes[28..36].try_into().expect("fixed slice"));
    let raw_len = usize::try_from(raw_len_u64).map_err(route_error)?;
    let raw_sha256 = bytes[36..68].try_into().expect("fixed slice");
    if record_count == 0
        || record_count > NATIVE_TACTIC_DECISION_COMPACTION_RECORDS
        || raw_len < NATIVE_TACTIC_DECISION_JOURNAL_HEADER_SIZE
        || raw_len > MAXIMUM_TACTIC_DECISION_SEGMENT_BYTES
    {
        return Err(route_message("tactic decision segment bounds are invalid"));
    }
    Ok(TacticDecisionSegmentHeader {
        start_index,
        record_count,
        raw_len,
        raw_sha256,
    })
}

pub(super) fn compacted_tactic_decision_count(
    seed_root: &Path,
) -> Result<u64, NativeTacticRouteRunError> {
    let mut next = 0_u64;
    for path in tactic_decision_segment_paths(seed_root)? {
        let header = read_tactic_decision_segment_header(&path)?;
        if header.start_index != next {
            return Err(route_message(
                "tactic decision segments are overlapping or detached",
            ));
        }
        next = next
            .checked_add(header.record_count)
            .ok_or_else(|| route_message("tactic decision segment count overflowed"))?;
    }
    Ok(next)
}

pub(super) fn read_compacted_tactic_decision_records(
    seed_root: &Path,
) -> Result<Vec<NativeTacticDecisionRecord>, NativeTacticRouteRunError> {
    let mut records: Vec<NativeTacticDecisionRecord> = Vec::new();
    for path in tactic_decision_segment_paths(seed_root)? {
        let bytes = fs::read(&path).map_err(route_error)?;
        let segment = decode_tactic_decision_segment(&bytes)?;
        if segment
            .first()
            .is_none_or(|record| record.decision_index != records.len() as u64)
        {
            return Err(route_message(
                "tactic decision segments are overlapping or detached",
            ));
        }
        records.extend(segment);
    }
    Ok(records)
}

pub(super) fn decode_tactic_decision_segment(
    bytes: &[u8],
) -> Result<Vec<NativeTacticDecisionRecord>, NativeTacticRouteRunError> {
    let header = decode_tactic_decision_segment_header(bytes)?;
    let raw = zstd::bulk::decompress(
        &bytes[NATIVE_TACTIC_DECISION_SEGMENT_HEADER_SIZE..],
        header.raw_len,
    )
    .map_err(route_error)?;
    if raw.len() != header.raw_len
        || <Sha256 as sha2::Digest>::digest(&raw)[..] != header.raw_sha256
    {
        return Err(route_message(
            "tactic decision segment payload digest is invalid",
        ));
    }
    let decoded = decode_tactic_decision_journal(&raw)?;
    if decoded.valid_bytes != raw.len()
        || decoded.records.len() as u64 != header.record_count
        || decoded
            .records
            .first()
            .is_none_or(|record| record.decision_index != header.start_index)
    {
        return Err(route_message("tactic decision segment payload is detached"));
    }
    Ok(decoded.records)
}

pub(super) fn compact_tactic_decision_journal(
    seed_root: &Path,
) -> Result<(), NativeTacticRouteRunError> {
    let path = tactic_decision_journal_path(seed_root);
    ensure_tactic_decision_journal(&path)?;
    let compacted_count = compacted_tactic_decision_count(seed_root)?;
    let bytes = fs::read(&path).map_err(route_error)?;
    let decoded = decode_tactic_decision_journal(&bytes)?;
    if decoded.valid_bytes != bytes.len() {
        return Err(route_message(
            "cannot compact a tactic decision journal with a truncated tail",
        ));
    }
    let records = decoded
        .records
        .into_iter()
        .filter(|record| record.decision_index >= compacted_count)
        .collect::<Vec<_>>();
    if records.is_empty() {
        rewrite_tactic_decision_journal(&path, &[])?;
        return Ok(());
    }
    if records[0].decision_index != compacted_count {
        return Err(route_message(
            "tactic decision journal is detached from its compacted segments",
        ));
    }
    let raw = encode_tactic_decision_journal(&records)?;
    if raw.len() > MAXIMUM_TACTIC_DECISION_SEGMENT_BYTES {
        return Err(route_message(
            "tactic decision segment exceeds its size bound",
        ));
    }
    let compressed = zstd::bulk::compress(&raw, NATIVE_TACTIC_DECISION_SEGMENT_COMPRESSION_LEVEL)
        .map_err(route_error)?;
    let start_index = records[0].decision_index;
    let end_index = start_index
        .checked_add(records.len() as u64)
        .ok_or_else(|| route_message("tactic decision segment index overflowed"))?;
    let mut segment =
        Vec::with_capacity(NATIVE_TACTIC_DECISION_SEGMENT_HEADER_SIZE + compressed.len());
    segment.extend_from_slice(NATIVE_TACTIC_DECISION_SEGMENT_MAGIC);
    segment.extend_from_slice(&NATIVE_TACTIC_DECISION_SEGMENT_VERSION.to_le_bytes());
    segment.extend_from_slice(&0_u16.to_le_bytes());
    segment.extend_from_slice(&start_index.to_le_bytes());
    segment.extend_from_slice(&(records.len() as u64).to_le_bytes());
    segment.extend_from_slice(&(raw.len() as u64).to_le_bytes());
    segment.extend_from_slice(&Sha256::digest(&raw));
    segment.extend_from_slice(&compressed);

    let root = tactic_decision_segments_root(seed_root);
    fs::create_dir_all(&root).map_err(route_error)?;
    let destination = root.join(format!("segment-{start_index:012}-{end_index:012}.dtqz"));
    if destination.exists() {
        if decode_tactic_decision_segment(&fs::read(&destination).map_err(route_error)?)? != records
        {
            return Err(route_message(
                "existing tactic decision segment conflicts with compaction",
            ));
        }
    } else {
        let partial = root.join(format!(
            ".segment-{start_index:012}-{end_index:012}.{}.partial",
            std::process::id()
        ));
        if partial.exists() {
            fs::remove_file(&partial).map_err(route_error)?;
        }
        write_new(&partial, &segment)?;
        fs::rename(&partial, &destination).map_err(route_error)?;
    }
    rewrite_tactic_decision_journal(&path, &[])
}

pub(super) fn rewrite_tactic_decision_journal(
    path: &Path,
    records: &[NativeTacticDecisionRecord],
) -> Result<(), NativeTacticRouteRunError> {
    let bytes = encode_tactic_decision_journal(records)?;
    let parent = path
        .parent()
        .ok_or_else(|| route_message("tactic decision journal has no parent"))?;
    let partial = parent.join(format!(
        ".{NATIVE_TACTIC_DECISION_JOURNAL_FILE}.{}.rewrite",
        std::process::id()
    ));
    if partial.exists() {
        fs::remove_file(&partial).map_err(route_error)?;
    }
    write_new(&partial, &bytes)?;
    if path.exists() {
        fs::remove_file(path).map_err(route_error)?;
    }
    fs::rename(&partial, path).map_err(route_error)
}

pub(super) fn encode_tactic_decision_journal(
    records: &[NativeTacticDecisionRecord],
) -> Result<Vec<u8>, NativeTacticRouteRunError> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(NATIVE_TACTIC_DECISION_JOURNAL_MAGIC);
    bytes.extend_from_slice(&NATIVE_TACTIC_DECISION_JOURNAL_VERSION.to_le_bytes());
    bytes.extend_from_slice(&0_u16.to_le_bytes());
    for record in records {
        bytes.extend_from_slice(&encode_tactic_decision_record(record)?);
    }
    Ok(bytes)
}

pub(super) fn encode_tactic_decision_record(
    decision: &NativeTacticDecisionRecord,
) -> Result<Vec<u8>, NativeTacticRouteRunError> {
    let payload = serde_cbor::to_vec(decision).map_err(route_error)?;
    if payload.len() > MAXIMUM_TACTIC_DECISION_RECORD_BYTES {
        return Err(route_message(
            "tactic decision journal record exceeds its size bound",
        ));
    }
    let payload_len = u32::try_from(payload.len()).map_err(route_error)?;
    let payload_sha256: [u8; 32] = Sha256::digest(&payload).into();
    let mut record = Vec::with_capacity(NATIVE_TACTIC_DECISION_RECORD_HEADER_SIZE + payload.len());
    record.extend_from_slice(&payload_len.to_le_bytes());
    record.extend_from_slice(&payload_sha256);
    record.extend_from_slice(&payload);
    Ok(record)
}

pub(super) fn decode_tactic_decision_journal(
    bytes: &[u8],
) -> Result<DecodedTacticDecisionJournal, NativeTacticRouteRunError> {
    if bytes.len() < NATIVE_TACTIC_DECISION_JOURNAL_HEADER_SIZE
        || &bytes[..8] != NATIVE_TACTIC_DECISION_JOURNAL_MAGIC
        || u16::from_le_bytes(bytes[8..10].try_into().expect("fixed slice"))
            != NATIVE_TACTIC_DECISION_JOURNAL_VERSION
        || u16::from_le_bytes(bytes[10..12].try_into().expect("fixed slice")) != 0
    {
        return Err(route_message("tactic decision journal header is invalid"));
    }
    let mut records: Vec<NativeTacticDecisionRecord> = Vec::new();
    let mut cursor = NATIVE_TACTIC_DECISION_JOURNAL_HEADER_SIZE;
    while cursor < bytes.len() {
        let remaining = bytes.len() - cursor;
        if remaining < NATIVE_TACTIC_DECISION_RECORD_HEADER_SIZE {
            break;
        }
        let payload_len =
            u32::from_le_bytes(bytes[cursor..cursor + 4].try_into().expect("fixed slice")) as usize;
        if payload_len > MAXIMUM_TACTIC_DECISION_RECORD_BYTES {
            return Err(route_message(
                "tactic decision journal record length is invalid",
            ));
        }
        let record_len = NATIVE_TACTIC_DECISION_RECORD_HEADER_SIZE
            .checked_add(payload_len)
            .ok_or_else(|| route_message("tactic decision journal record length overflows"))?;
        if remaining < record_len {
            break;
        }
        let expected_sha256: [u8; 32] = bytes[cursor + 4..cursor + 36]
            .try_into()
            .expect("fixed slice");
        let payload =
            &bytes[cursor + NATIVE_TACTIC_DECISION_RECORD_HEADER_SIZE..cursor + record_len];
        let actual_sha256: [u8; 32] = Sha256::digest(payload).into();
        if actual_sha256 != expected_sha256 {
            return Err(route_message(
                "tactic decision journal record digest is invalid",
            ));
        }
        let mut deserializer = serde_cbor::Deserializer::from_slice(payload);
        let decision =
            NativeTacticDecisionRecord::deserialize(&mut deserializer).map_err(route_error)?;
        deserializer.end().map_err(route_error)?;
        let expected_index = records.first().map_or(decision.decision_index, |first| {
            first.decision_index + records.len() as u64
        });
        if decision.decision_index != expected_index {
            return Err(route_message(
                "tactic decision journal record index is detached",
            ));
        }
        records.push(decision);
        cursor += record_len;
    }
    Ok(DecodedTacticDecisionJournal {
        records,
        valid_bytes: cursor,
    })
}
