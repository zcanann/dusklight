pub(super) use super::campaign_schedule::{
    ActiveIncumbentContinuation, ActiveTerminalRefinementRollout, first_demonstration_intervention,
    select_incumbent_rejoin_offset, should_probe_policy_before_branch,
    should_start_paired_terminal_return,
};
use super::paired_terminal_returns::{ActivePairedTerminalReturn, PairedTerminalReturnSeed};
use super::*;

mod seed_finalization;
use seed_finalization::finalize_seed;
mod seed_initialization;
use seed_initialization::{SeedCampaignInitialization, load_or_create_seed_campaign};

pub(super) const NATIVE_TACTIC_RESULT_ADMISSION_SCHEMA_V1: &str =
    "dusklight-native-tactic-result-admission/v1";

fn incumbent_continuation_for_exact_state(
    campaign: &TacticQCampaign,
    route_checkpoint_sha256: Digest,
    state_sha256: Digest,
    executed_actions: &[(&str, bool)],
) -> Result<Option<ActiveIncumbentContinuation>, NativeTacticRouteRunError> {
    let continuation = campaign
        .state_graph()
        .map_err(route_error)?
        .exact_terminal_continuation(crate::state_graph::ExactStateId {
            route_checkpoint_sha256,
            state_sha256,
        });
    match continuation {
        Ok(Some(continuation)) => Ok(ActiveIncumbentContinuation::new(
            continuation.terminal_route_checkpoint_sha256,
            executed_actions,
        )),
        Ok(None) => Ok(None),
        Err(crate::state_graph::StateGraphError::Invalid(_)) => Ok(None),
        Err(error) => Err(route_error(error)),
    }
}

fn incumbent_continuation_for_branch(
    campaign: &TacticQCampaign,
    branch: &crate::tactic_q_campaign::TacticCampaignBranch,
) -> Result<Option<ActiveIncumbentContinuation>, NativeTacticRouteRunError> {
    incumbent_continuation_for_exact_state(
        campaign,
        branch.logical_frontier.identity_sha256,
        branch.logical_frontier.state_sha256,
        &[],
    )
}

fn recover_incumbent_continuation(
    campaign: &TacticQCampaign,
    trace: &[NativeTacticDecisionTrace],
) -> Result<Option<ActiveIncumbentContinuation>, NativeTacticRouteRunError> {
    let Some(last) = trace.last() else {
        return Ok(None);
    };
    let episode_start = trace
        .iter()
        .rposition(|decision| decision.episode != last.episode)
        .map_or(0, |index| index + 1);
    let first = &trace[episode_start];
    let executed_actions = trace[episode_start..]
        .iter()
        .zip(&campaign.replay()[episode_start..])
        .map(|(decision, transition)| {
            (
                decision.selected_option_id.as_str(),
                transition.execution.end_reason == OptionEndReason::Terminated,
            )
        })
        .collect::<Vec<_>>();
    incumbent_continuation_for_exact_state(
        campaign,
        first.frontier_identity,
        first.before.snapshot_sha256,
        &executed_actions,
    )
}

fn recover_active_episode_ticks(
    campaign: &TacticQCampaign,
    trace: &[NativeTacticDecisionTrace],
) -> Result<u64, NativeTacticRouteRunError> {
    let Some(last) = trace.last() else {
        return Ok(0);
    };
    let episode_start = trace
        .iter()
        .rposition(|decision| decision.episode != last.episode)
        .map_or(0, |index| index + 1);
    campaign.replay()[episode_start..]
        .iter()
        .try_fold(0_u64, |ticks, transition| {
            ticks
                .checked_add(u64::from(transition.execution.duration.realized_ticks))
                .ok_or_else(|| route_message("active episode tick count overflowed"))
        })
}

fn incumbent_continuation_suffix(
    campaign: &TacticQCampaign,
    continuation: ActiveIncumbentContinuation,
    encoder: &GoalConditionedTacticFeatureEncoder,
) -> Result<Option<InputTape>, NativeTacticRouteRunError> {
    let graph = campaign.state_graph().map_err(route_error)?;
    let terminal_route = graph
        .route(continuation.terminal_route_checkpoint_sha256())
        .ok_or_else(|| route_message("incumbent continuation terminal route disappeared"))?;
    let current_frames = campaign.route_tape.frames.len();
    if current_frames >= terminal_route.frames.len() {
        return Ok(None);
    }
    let query = encoder
        .encode(&campaign.current.snapshot)
        .map_err(route_error)?;
    let candidates = graph
        .nodes()
        .filter(|node| node.restoration.executable && !node.terminal)
        .filter(|node| {
            node.state.world.stage == campaign.current.snapshot.world.stage
                && node.state.world.room == campaign.current.snapshot.world.room
        })
        .filter_map(|node| {
            let route = graph.route(node.id.route_checkpoint_sha256)?;
            let offset = route.frames.len();
            (offset >= current_frames
                && offset < terminal_route.frames.len()
                && route.boot == terminal_route.boot
                && route.tick_rate_numerator == terminal_route.tick_rate_numerator
                && route.tick_rate_denominator == terminal_route.tick_rate_denominator
                && terminal_route.frames.starts_with(&route.frames))
            .then_some((offset, node))
        })
        .map(|(offset, node)| {
            encoder
                .encode(&node.state)
                .map(|features| (offset, features))
                .map_err(route_error)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let rejoin_offset =
        select_incumbent_rejoin_offset(&query, &candidates, &encoder.distance_weights())
            .unwrap_or(current_frames);
    let frames = terminal_route
        .frames
        .get(rejoin_offset..)
        .ok_or_else(|| route_message("incumbent rejoin offset exceeds its terminal route"))?;
    if frames.is_empty() {
        return Ok(None);
    }
    let mut tape = InputTape {
        tick_rate_numerator: terminal_route.tick_rate_numerator,
        tick_rate_denominator: terminal_route.tick_rate_denominator,
        ..InputTape::default()
    };
    tape.frames.extend_from_slice(frames);
    tape.validate().map_err(route_error)?;
    Ok(Some(tape))
}

pub(super) fn simplify_incumbent_rejoin_coordinates(
    current: [f32; 3],
    lineage: &[[f32; 3]],
) -> Vec<[f32; 3]> {
    const MAXIMUM_COORDINATES: usize =
        dusklight_control::controller_program::MAX_SEEK_COORDINATE_SEQUENCE_POINTS;
    let mut points = Vec::with_capacity(lineage.len().saturating_add(1));
    points.push(current);
    for point in lineage {
        if points
            .last()
            .is_none_or(|previous| (previous[0] - point[0]).hypot(previous[2] - point[2]) > 1.0e-3)
        {
            points.push(*point);
        }
    }
    if points.len() <= 1 {
        return Vec::new();
    }

    let mut selected = vec![0, points.len() - 1];
    while selected.len() < MAXIMUM_COORDINATES.saturating_add(1) {
        selected.sort_unstable();
        let mut best: Option<(f32, usize)> = None;
        for endpoints in selected.windows(2) {
            let start_index = endpoints[0];
            let end_index = endpoints[1];
            let start = points[start_index];
            let end = points[end_index];
            let dx = end[0] - start[0];
            let dz = end[2] - start[2];
            let length_squared = dx * dx + dz * dz;
            for index in start_index + 1..end_index {
                let point = points[index];
                let progress = if length_squared > 0.0 {
                    (((point[0] - start[0]) * dx + (point[2] - start[2]) * dz) / length_squared)
                        .clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let nearest_x = start[0] + dx * progress;
                let nearest_z = start[2] + dz * progress;
                let distance_squared =
                    (point[0] - nearest_x).powi(2) + (point[2] - nearest_z).powi(2);
                if best.is_none_or(|(best_distance, best_index)| {
                    distance_squared > best_distance
                        || (distance_squared == best_distance && index < best_index)
                }) {
                    best = Some((distance_squared, index));
                }
            }
        }
        let Some((distance_squared, index)) = best else {
            break;
        };
        if distance_squared <= 1.0e-6 {
            break;
        }
        selected.push(index);
    }
    selected.sort_unstable();
    selected
        .into_iter()
        .skip(1)
        .map(|index| points[index])
        .collect()
}

fn incumbent_rejoin_intermediate_tolerance(current: [f32; 3], coordinates: &[[f32; 3]]) -> f32 {
    let mut previous = current;
    let minimum_spacing = coordinates
        .iter()
        .fold(f32::INFINITY, |minimum, coordinate| {
            let spacing = (coordinate[0] - previous[0]).hypot(coordinate[2] - previous[2]);
            previous = *coordinate;
            minimum.min(spacing)
        });
    (minimum_spacing * 0.25).clamp(8.0, 96.0)
}

fn incumbent_rejoin_targets(
    campaign: &TacticQCampaign,
    continuation: ActiveIncumbentContinuation,
) -> Result<Vec<IncumbentRejoinTarget>, NativeTacticRouteRunError> {
    let graph = campaign.state_graph().map_err(route_error)?;
    let terminal_route = graph
        .route(continuation.terminal_route_checkpoint_sha256())
        .ok_or_else(|| route_message("incumbent rejoin terminal route disappeared"))?;
    let current_frames = campaign.route_tape.frames.len();
    let current_position = campaign
        .current
        .snapshot
        .player
        .position_f32_bits
        .map(f32::from_bits);
    if current_position.iter().any(|value| !value.is_finite()) {
        return Err(route_message("incumbent rejoin source position is invalid"));
    }
    let mut lineage = graph
        .nodes()
        .filter(|node| node.restoration.executable && !node.terminal)
        .filter(|node| {
            node.state.world.stage == campaign.current.snapshot.world.stage
                && node.state.world.room == campaign.current.snapshot.world.room
        })
        .filter_map(|node| {
            let route = graph.route(node.id.route_checkpoint_sha256)?;
            let offset = route.frames.len();
            (offset < terminal_route.frames.len()
                && route.boot == terminal_route.boot
                && route.tick_rate_numerator == terminal_route.tick_rate_numerator
                && route.tick_rate_denominator == terminal_route.tick_rate_denominator
                && terminal_route.frames.starts_with(&route.frames))
            .then_some((
                offset,
                node.state.player.position_f32_bits.map(f32::from_bits),
            ))
        })
        .collect::<Vec<_>>();
    lineage.sort_by_key(|(offset, _)| *offset);
    let mut ranked = lineage
        .iter()
        .enumerate()
        .filter_map(|(index, (offset, position))| {
            let headroom = offset.checked_sub(current_frames)?;
            if headroom <= 1 || position.iter().any(|value| !value.is_finite()) {
                return None;
            }
            let planar_distance =
                (position[0] - current_position[0]).hypot(position[2] - current_position[2]);
            let boundary_spacing = index
                .checked_sub(1)
                .and_then(|previous| lineage.get(previous))
                .map(|(_, previous)| (position[0] - previous[0]).hypot(position[2] - previous[2]))
                .unwrap_or(32.0);
            let tolerance = (boundary_spacing * 0.25).clamp(8.0, 32.0);
            let maximum_ticks = headroom
                .saturating_sub(1)
                .min(dusklight_learning::native_generic_tactic::MAX_NATIVE_TACTIC_TICKS as usize)
                as u32;
            let path = lineage[..=index]
                .iter()
                .filter(|(candidate_offset, _)| *candidate_offset > current_frames)
                .map(|(_, coordinate)| *coordinate)
                .collect::<Vec<_>>();
            let coordinates = simplify_incumbent_rejoin_coordinates(current_position, &path);
            let intermediate_tolerance =
                incumbent_rejoin_intermediate_tolerance(current_position, &coordinates);
            (maximum_ticks > 0).then_some((
                planar_distance / headroom as f32,
                headroom,
                IncumbentRejoinTarget {
                    coordinates,
                    intermediate_tolerance,
                    tolerance,
                    maximum_ticks,
                },
            ))
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        left.0
            .total_cmp(&right.0)
            .then_with(|| right.1.cmp(&left.1))
            .then_with(|| {
                let left_coordinate = left.2.coordinates.last().expect("ranked rejoin target");
                let right_coordinate = right.2.coordinates.last().expect("ranked rejoin target");
                left_coordinate[0]
                    .total_cmp(&right_coordinate[0])
                    .then_with(|| left_coordinate[2].total_cmp(&right_coordinate[2]))
            })
    });
    ranked.truncate(16);
    Ok(ranked.into_iter().map(|(_, _, target)| target).collect())
}

fn force_incumbent_candidates(
    batch: &mut TacticQProposalBatch,
    descriptors: &[OptionActionDescriptor],
) -> Result<(), NativeTacticRouteRunError> {
    if descriptors.is_empty()
        || descriptors.iter().any(|descriptor| {
            !batch
                .ranking
                .choices
                .iter()
                .any(|choice| choice.applicable && choice.descriptor == *descriptor)
        })
    {
        return Err(route_message(
            "incumbent candidate is absent from its applicable action surface",
        ));
    }
    let selected_template = batch
        .proposals
        .first()
        .cloned()
        .ok_or_else(|| route_message("incumbent candidate has no proposal template"))?;
    let mut ordered = batch
        .ranking
        .values
        .ranked
        .iter()
        .map(|ranked| &ranked.descriptor)
        .filter(|descriptor| descriptors.contains(descriptor))
        .cloned()
        .collect::<Vec<_>>();
    for descriptor in descriptors {
        if !ordered.contains(descriptor) {
            ordered.push(descriptor.clone());
        }
    }
    batch.proposals = ordered
        .into_iter()
        .map(|descriptor| {
            let mut selected = selected_template.clone();
            selected.descriptor = descriptor;
            selected.reason = TacticSelectionReason::TerminalCostRefinement;
            selected
        })
        .collect();
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn run_seed(
    config: &NativeTacticRouteRunConfig<'_>,
    pool: &NativeTacticProposalPool,
    registry: &FactRegistry,
    encoder: &GoalConditionedTacticFeatureEncoder,
    reward_spec: &TacticRewardSpec,
    initial_facts: &FactSnapshot,
    route_prefix: &InputTape,
    action_schema_sha256: Digest,
    promoted_tactics: &[ImportedPromotedTactic],
    root_checkpoint_sha256: Digest,
    root_tape_ref: StoredContentRef,
    shared_content_store: TacticQContentStore,
    inherited_learner_snapshot: Arc<TacticQImmutableLearnerSnapshot>,
    live_learner: Option<SharedTacticLearnerAuthority>,
    cross_lane_replay: Option<Arc<CrossLaneReplayCoordinator>>,
    seed_index: usize,
    seed: u64,
) -> Result<CompletedNativeTacticSeed, NativeTacticRouteRunError> {
    let invocation_started = Instant::now();
    let execution_plan_sha256 = config.execution_plan.identity()?;
    let lane = config
        .execution_plan
        .lanes
        .get(seed_index)
        .filter(|lane| lane.seed == seed)
        .ok_or_else(|| route_message("tactic seed is detached from its execution-plan lane"))?;
    let seed_root = config
        .output_root
        .join(format!("seed-{seed_index:03}-{seed}"));
    let seed_root_preexisted = seed_root.exists();
    let resuming_seed =
        seed_root_preexisted && config.resume && has_tactic_recovery_point(&seed_root)?;
    let source_frame = config.optimization.route.source_boundary_index;
    let horizon = config.optimization.budgets.exploration_horizon_ticks;
    let maximum_tactic_ticks = goal_tactic_maximum_ticks(horizon)?;
    let SeedCampaignInitialization {
        mut campaign,
        mut trace,
        mut selection_counts,
        mut native_ticks,
        mut episode,
    } = load_or_create_seed_campaign(
        config,
        lane,
        &seed_root,
        encoder,
        root_checkpoint_sha256,
        seed_index,
        seed,
        &inherited_learner_snapshot,
        initial_facts,
        registry,
        maximum_tactic_ticks,
        action_schema_sha256,
        promoted_tactics,
        route_prefix,
        execution_plan_sha256,
        resuming_seed,
        seed_root_preexisted,
    )?;
    let source_lane = TacticMacroSourceLane { seed_index, seed };
    let recovered_candidates = load_active_tactic_candidates(config.output_root, source_lane)?;
    let mut active_tactics = promoted_tactics.to_vec();
    let mut active_candidate_discovered = !recovered_candidates.is_empty();
    merge_promoted_tactic_entries(&mut active_tactics, recovered_candidates)?;
    let performance = if resuming_seed {
        load_seed_performance(&seed_root, campaign.decision_index)?
    } else {
        NativeTacticSeedPerformance {
            schema: TACTIC_ROUTE_PERFORMANCE_SCHEMA_V2.into(),
            decisions: 0,
            useful_decisions: 0,
            native_restore_accounting: NativeTacticRestoreAccounting::default(),
            timing: NativeTacticRouteTiming::default(),
        }
    };
    let mut timing = performance.timing;
    if timing.wall_micros == 0
        && timing.orchestration_micros == 0
        && timing.orchestration_breakdown.is_none()
    {
        timing.orchestration_breakdown = Some(NativeTacticOrchestrationTiming::default());
    }
    if timing.persistence_micros > 0 && timing.persistence_breakdown.is_none() {
        timing.persistence_breakdown = Some(NativeTacticPersistenceTiming {
            unattributed_micros: timing.persistence_micros,
            ..NativeTacticPersistenceTiming::default()
        });
    }
    if resuming_seed {
        reconcile_recovered_seed_timing(&mut timing)?;
    }
    let setup_top_baseline = ExclusiveTopTimingSnapshot::capture(&timing);
    let mut native_restore_accounting = performance.native_restore_accounting;
    let prior_wall_micros = timing.wall_micros;
    let prior_model_update_micros = timing.model_update_micros;
    let traced_useful_decisions = trace
        .iter()
        .filter(|decision| decision_trace_is_useful(decision))
        .count() as u64;
    if resuming_seed && performance.useful_decisions != traced_useful_decisions {
        return Err(route_message(
            "native tactic recovery performance is detached from its decision journal",
        ));
    }
    let mut useful_decisions = traced_useful_decisions;
    let maximum_tactic_ticks = u64::from(maximum_tactic_ticks);
    let encode = |facts: &FactSnapshot| encoder.encode(facts);
    let mut online = TacticQOnlineLearningController::default();
    let demonstration_curriculum = lane.role == NativeTacticLaneRole::TerminalSupport
        && lane.intervention == NativeTacticInterventionPlan::DemonstrationFrontierOnce
        && inherited_learner_snapshot
            .training_corpus()
            .episode_groups
            .contains(&TACTIC_Q_DEMONSTRATION_EPISODE_GROUP);
    let demonstration_frontier_states = if demonstration_curriculum {
        campaign.demonstration_frontier_state_sha256s()
    } else {
        BTreeSet::new()
    };
    let mut covered_demonstration_frontiers = trace
        .iter()
        .filter(|decision| decision.branch_acquisition.is_some())
        .map(|decision| decision.before.snapshot_sha256)
        .filter(|state| demonstration_frontier_states.contains(state))
        .collect::<BTreeSet<_>>();
    let checkpoint_content_root = tactic_content_store_path(&seed_root);
    let decision_content_store = if shared_content_store.root() == checkpoint_content_root {
        shared_content_store
    } else {
        TacticQContentStore::initialize(&checkpoint_content_root).map_err(route_error)?
    };
    // Process-local handles intentionally do not survive campaign resume. A
    // running native worker does, however, own a bounded cache with more than
    // one image; preserve every identity that can still be selected by the
    // logical graph instead of remembering only the latest endpoint.
    let mut cached_frontiers = RetainedNativeTacticFrontiers::new(TACTIC_CHECKPOINT_CACHE_ENTRIES);
    let mut branch_acquisition: Option<TacticFrontierAcquisition> = None;
    let mut active_incumbent_continuation = recover_incumbent_continuation(&campaign, &trace)?;
    let mut active_episode_ticks = recover_active_episode_ticks(&campaign, &trace)?;
    let mut active_paired_terminal_return = ActivePairedTerminalReturn::recover(&trace)?;
    let mut active_terminal_refinement = active_paired_terminal_return
        .as_ref()
        .map(ActivePairedTerminalReturn::rollout)
        .or_else(|| {
            trace
                .iter()
                .rev()
                .find(|decision| decision.branch_acquisition.is_some())
                .and_then(|decision| decision.branch_acquisition.as_ref())
                .and_then(|acquisition| {
                    ActiveTerminalRefinementRollout::new(
                        acquisition.replayed_prefix_ticks,
                        acquisition.exact_terminal_ticks_to_go,
                    )
                })
        });
    let mut demonstration_intervention_pending = false;
    let retained_success_root = seed_root.join("retained-successes");
    let mut best_success = load_best_retained_success(
        &retained_success_root,
        execution_plan_sha256,
        campaign.objective_sha256,
        campaign.root_checkpoint_sha256,
    )?;
    synchronize_graph_terminal_result(
        &retained_success_root,
        campaign.decision_index,
        &campaign,
        &mut best_success,
    )?;
    let mut consumed_learner_snapshot = inherited_learner_snapshot;
    if let Some(pair) = active_paired_terminal_return.as_ref()
        && consumed_learner_snapshot.sha256 != pair.frozen_learner_snapshot_sha256()
    {
        let learner = live_learner.as_ref().ok_or_else(|| {
            route_message("paired terminal-return recovery has no learner authority")
        })?;
        let frozen = lock_learner_authority(learner)?.snapshot_by_identity(
            pair.frozen_learner_snapshot_sha256(),
            pair.frozen_replay_revision(),
        )?;
        campaign
            .consume_learner_snapshot_with_exploration_filter(&frozen, |episode_group| {
                lane.owns_episode_group(episode_group)
            })
            .map_err(route_error)?;
        consumed_learner_snapshot = frozen;
    }
    let mut replay_session = build_replay_session(
        config.execution_plan,
        live_learner,
        lane,
        consumed_learner_snapshot.replay_revision,
        cross_lane_replay,
    )?;
    if let Some(session) = replay_session.as_ref() {
        session.synchronize_startup(
            campaign.decision_index,
            resuming_seed.then_some((&campaign, trace.as_slice())),
        )?;
    }
    let mut lease_ledger = NativeTacticLeaseLedger::open(&seed_root)?;
    if resuming_seed {
        let completed_expansions_by_decision = trace
            .iter()
            .filter_map(|decision| {
                decision
                    .scheduler_decision
                    .as_ref()
                    .map(|scheduler| scheduler.evaluated_expansion_sha256.clone())
                    .or_else(|| {
                        decision
                            .policy_evaluation_decision
                            .as_ref()
                            .map(|evaluation| evaluation.evaluated_expansion_sha256.clone())
                    })
                    .map(|expansions| (decision.decision_index, expansions))
            })
            .collect::<BTreeMap<_, _>>();
        lease_ledger.reconcile_unresolved(&completed_expansions_by_decision)?;
    }
    let completed_trace_dispatches = trace.iter().try_fold(0_u64, |total, decision| {
        total
            .checked_add(u64::try_from(decision.proposal_batch.len()).map_err(route_error)?)
            .ok_or_else(|| route_message("completed tactic dispatch count overflowed"))
    })?;
    if lease_ledger.accounting()?.completed_leases != completed_trace_dispatches {
        return Err(route_message(
            "native tactic lease journal is detached from the completed decision trace",
        ));
    }
    if resuming_seed {
        prune_tactic_partial_finalization(&seed_root)?;
        prune_tactic_native_attempts(&seed_root, campaign.decision_index)?;
        prune_tactic_recovery_points(&seed_root, campaign.decision_index)?;
    } else {
        let recovery_decision_index = campaign.decision_index;
        let initial_recovery_started = Instant::now();
        persist_tactic_recovery_point(
            &seed_root,
            &mut campaign,
            &decision_content_store,
            NativeTacticSeedPerformance {
                schema: TACTIC_ROUTE_PERFORMANCE_SCHEMA_V2.into(),
                decisions: recovery_decision_index,
                useful_decisions,
                native_restore_accounting: native_restore_accounting.clone(),
                timing: timing.clone(),
            },
        )?;
        let initial_recovery_micros = elapsed_micros(initial_recovery_started.elapsed());
        record_persistence_timing(
            &mut timing,
            NativeTacticPersistenceTiming {
                recovery_checkpoint_micros: initial_recovery_micros,
                ..NativeTacticPersistenceTiming::default()
            },
        )?;
    }
    let mut decision_journal = TacticDecisionJournalAppender::open(&seed_root)?;
    if decision_journal.next_decision_index() != campaign.decision_index {
        return Err(route_message(
            "tactic decision journal cursor is detached from campaign recovery",
        ));
    }

    let seed_setup_wall_micros = elapsed_micros(invocation_started.elapsed());
    let seed_setup_known_top_micros = setup_top_baseline.checked_delta_total(&timing)?;
    let seed_setup_orchestration_micros = seed_setup_wall_micros
        .checked_sub(seed_setup_known_top_micros)
        .ok_or_else(|| route_message("native tactic seed setup timing is detached"))?;
    record_orchestration_detail(
        &mut timing,
        OrchestrationPhase::SeedSetup,
        seed_setup_orchestration_micros,
    )?;
    record_orchestration_total(&mut timing, seed_setup_orchestration_micros)?;

    while campaign.decision_index < config.execution_plan.budgets.decisions_per_lane
        && native_ticks < config.optimization.budgets.simulated_tick_budget
        && !config
            .execution_plan
            .budgets
            .native_ticks
            .reached(native_ticks)
        && !config
            .execution_plan
            .budgets
            .wall_micros
            .reached(prior_wall_micros.saturating_add(elapsed_micros(invocation_started.elapsed())))
    {
        let iteration_started = Instant::now();
        let iteration_top_baseline = ExclusiveTopTimingSnapshot::capture(&timing);
        let iteration_orchestration_detail_baseline = orchestration_detail_total(&timing)?;
        if cancellation_requested(config) {
            return Err(route_cancelled("native tactic route paused"));
        }
        if let Some(session) = replay_session.as_mut() {
            session.begin_decision(campaign.decision_index)?;
        }
        if active_paired_terminal_return
            .as_ref()
            .is_some_and(ActivePairedTerminalReturn::control_pending)
        {
            let branch_started = Instant::now();
            episode = episode
                .checked_add(1)
                .ok_or_else(|| route_message("episode counter overflowed"))?;
            let control_target = active_paired_terminal_return
                .as_ref()
                .expect("checked paired control")
                .control_target();
            let control_branch = campaign
                .exact_frontier_branch(control_target)
                .map_err(route_error)?;
            let action_catalog_started = Instant::now();
            let branch_proposals = parameterized_catalog_for_state_with_promoted(
                seed,
                campaign.decision_index,
                &control_branch.state,
                encoder,
                u32::try_from(maximum_tactic_ticks).map_err(route_error)?,
                parameterized_feedback_for_state(&campaign, &control_branch.state, encoder)?,
                action_schema_sha256,
                &active_tactics,
            )?;
            record_orchestration_detail(
                &mut timing,
                OrchestrationPhase::ActionCatalogConstruction,
                elapsed_micros(action_catalog_started.elapsed()),
            )?;
            campaign
                .restore_branch(
                    &control_branch,
                    lane.episode_group(episode)?,
                    registry,
                    &branch_proposals.catalog,
                    &branch_proposals.blueprints,
                    |_| true,
                )
                .map_err(route_error)?;
            let pair = active_paired_terminal_return
                .as_mut()
                .expect("checked paired control");
            pair.begin_control();
            active_terminal_refinement = Some(pair.rollout());
            active_incumbent_continuation = None;
            active_episode_ticks = 0;
            branch_acquisition = None;
            timing.checkpoint_branching_micros = timing
                .checkpoint_branching_micros
                .saturating_add(elapsed_micros(branch_started.elapsed()));
        }
        let decision_acquisition_rank = lane.acquisition.rank(episode);
        let mut active_acquisition_rank = if active_paired_terminal_return.is_some() {
            0
        } else {
            decision_acquisition_rank
        };
        let terminal_restart = campaign.current.snapshot.terminal.reached == Some(true);
        if terminal_restart
            && config.execution_plan.proposal_policy == TacticProposalPolicy::Learned
            && active_paired_terminal_return.is_none()
            && !active_candidate_discovered
        {
            let candidates = discover_active_tactic_candidates(
                config.output_root,
                source_lane,
                encoder,
                campaign.decision_index,
            )?;
            active_candidate_discovered = !candidates.is_empty();
            merge_promoted_tactic_entries(&mut active_tactics, candidates)?;
        }
        let mut policy_update_probes = Vec::new();
        if should_probe_policy_before_branch(terminal_restart)
            && active_paired_terminal_return.is_none()
            && let Some(session) = replay_session.as_mut()
        {
            let learner_refresh_started = Instant::now();
            if let Some(snapshot) = session.pending_snapshot()? {
                let fixed_feedback = parameterized_feedback_for_state(
                    &campaign,
                    &campaign.current.snapshot,
                    encoder,
                )?;
                let fixed = parameterized_catalog_for_state_with_promoted(
                    seed,
                    campaign.decision_index,
                    &campaign.current.snapshot,
                    encoder,
                    u32::try_from(maximum_tactic_ticks).map_err(route_error)?,
                    fixed_feedback,
                    action_schema_sha256,
                    &active_tactics,
                )?;
                let fixed_catalog = Arc::new(fixed.catalog);
                let fixed_blueprints = Arc::new(fixed.blueprints);
                policy_update_probes.push(consume_policy_update_with_probe(
                    session,
                    &mut campaign,
                    &consumed_learner_snapshot,
                    &snapshot,
                    None,
                    PolicyUpdateProbeContext {
                        catalog: &fixed_catalog,
                        blueprints: &fixed_blueprints,
                        action_schema_sha256,
                        encoder,
                        maximum_proposals: config.execution_plan.proposal_width_per_decision,
                        acquisition_partition: decision_acquisition_rank,
                        proposal_policy: config.execution_plan.proposal_policy,
                        force_exploration: false,
                    },
                )?);
                consumed_learner_snapshot = snapshot;
            }
            record_orchestration_detail(
                &mut timing,
                OrchestrationPhase::LearnerRefresh,
                elapsed_micros(learner_refresh_started.elapsed()),
            )?;
        }
        let next_episode = episode
            .checked_add(1)
            .ok_or_else(|| route_message("episode counter overflowed"))?;
        let next_branch_acquisition_rank = lane.acquisition.rank(next_episode);
        let post_terminal_discovery_tick_budget = campaign
            .best_graph_terminal_path()
            .map_err(route_error)?
            .and_then(|path| {
                lane.acquisition
                    .post_terminal_discovery_tick_budget(path.root_to_terminal_ticks)
            });
        let demonstration_coverage_pending = demonstration_curriculum
            && covered_demonstration_frontiers.len() < demonstration_frontier_states.len();
        let current_route_ticks = u64::try_from(campaign.route_tape.frames.len())
            .map_err(|_| route_message("terminal refinement route length overflows"))?
            .saturating_sub(source_frame);
        let terminal_refinement_completed = active_terminal_refinement
            .is_some_and(|rollout| !rollout.has_remaining_budget(current_route_ticks));
        if terminal_refinement_completed {
            active_terminal_refinement = None;
        }
        if active_incumbent_continuation
            .is_some_and(ActiveIncumbentContinuation::should_execute_rejoin)
            && incumbent_rejoin_targets(
                &campaign,
                active_incumbent_continuation.expect("checked incumbent continuation"),
            )?
            .is_empty()
        {
            active_incumbent_continuation
                .as_mut()
                .expect("checked incumbent continuation")
                .skip_rejoin()
                .map_err(route_message)?;
        }
        if active_incumbent_continuation
            .is_some_and(ActiveIncumbentContinuation::should_execute_suffix)
            && incumbent_continuation_suffix(
                &campaign,
                active_incumbent_continuation.expect("checked incumbent continuation"),
                encoder,
            )?
            .is_none()
        {
            active_incumbent_continuation
                .as_mut()
                .expect("checked incumbent continuation")
                .skip_suffix()
                .map_err(route_message)?;
        }
        let incumbent_continuation_completed = active_incumbent_continuation
            .is_some_and(ActiveIncumbentContinuation::candidate_completed);
        let terminal_refinement_in_progress = active_terminal_refinement.is_some()
            || active_incumbent_continuation
                .is_some_and(|continuation| !continuation.candidate_completed());
        let maximum_frontier_frames = usize::try_from(
            source_frame.saturating_add(horizon.saturating_sub(maximum_tactic_ticks)),
        )
        .map_err(route_error)?;
        let graph_scheduling_started = Instant::now();
        let preferred_restoration_targets = cached_frontiers.pending_locality_targets();
        let continuation = if campaign.replay().is_empty() {
            None
        } else {
            online
                .continue_rollout(
                    &mut campaign,
                    TacticQOnlineRolloutRequest {
                        force_branch: incumbent_continuation_completed,
                        active_acquisition_rank,
                        next_acquisition_rank: next_branch_acquisition_rank,
                        current_rollout_ticks: active_episode_ticks,
                        post_terminal_discovery_tick_budget,
                        demonstration_coverage_pending,
                        terminal_refinement_in_progress,
                        terminal_refinement_completed,
                        root_refresh_due: lane.root_refresh_due(
                            next_episode,
                            config.execution_plan.root_refresh_cadence,
                        ),
                        goal_relabeling_enabled: config
                            .execution_plan
                            .value_treatment
                            .uses_goal_relabeling(),
                        terminal_frontier_action_value_enabled: config
                            .execution_plan
                            .value_treatment
                            .uses_terminal_frontier_action_value(),
                        seed,
                        round: frontier_sampling_round(next_episode),
                        episode_group: lane.episode_group(next_episode)?,
                        maximum_route_frames: maximum_frontier_frames,
                        goal_distance_feature: encoder.goal_distance_feature(),
                    },
                    registry,
                    &[],
                    &preferred_restoration_targets,
                    &encode,
                    &|campaign, state| {
                        parameterized_action_surface_for_state(
                            campaign,
                            registry,
                            seed,
                            campaign.decision_index,
                            state,
                            encoder,
                            u32::try_from(maximum_tactic_ticks).map_err(|error| {
                                TacticQCampaignError::Frontier(error.to_string())
                            })?,
                            action_schema_sha256,
                            &active_tactics,
                        )
                        .map_err(|error| TacticQCampaignError::Frontier(error.to_string()))
                    },
                )
                .map_err(route_error)?
        };
        let graph_scheduling_micros = elapsed_micros(graph_scheduling_started.elapsed());
        record_orchestration_detail(
            &mut timing,
            OrchestrationPhase::GraphSchedulingAndLeasing,
            graph_scheduling_micros,
        )?;
        if let Some(continuation) = continuation {
            episode = next_episode;
            let branch_acquisition_context = continuation.continuation;
            active_acquisition_rank = branch_acquisition_context.acquisition_rank;
            let demonstration_branch = branch_acquisition_context.demonstration;
            let selected_branch = continuation.branch;
            active_incumbent_continuation =
                incumbent_continuation_for_branch(&campaign, &selected_branch)?;
            active_episode_ticks = 0;
            let selected_uncovered_demonstration_frontier = demonstration_branch
                && demonstration_frontier_states
                    .contains(&selected_branch.logical_frontier.state_sha256)
                && !covered_demonstration_frontiers
                    .contains(&selected_branch.logical_frontier.state_sha256);
            demonstration_intervention_pending = first_demonstration_intervention(
                demonstration_coverage_pending,
                branch_acquisition_context.prefer_root,
                selected_uncovered_demonstration_frontier,
            );
            branch_acquisition = selected_branch.acquisition.clone();
            active_terminal_refinement = branch_acquisition.as_ref().and_then(|acquisition| {
                ActiveTerminalRefinementRollout::new(
                    acquisition.replayed_prefix_ticks,
                    acquisition.exact_terminal_ticks_to_go,
                )
            });
            timing.checkpoint_branching_micros = timing
                .checkpoint_branching_micros
                .saturating_add(graph_scheduling_micros);
        }

        // Reserve horizon for the tactic Q actually selected at this state,
        // not for the longest unrelated entry in the catalog. This lets short
        // tactics compose beyond `horizon - catalog_maximum` while still
        // branching before any selected tactic could exceed the bound.
        //
        // Restoring a branch can change the selected tactic. Recheck until the
        // preview fits; the periodic root sample guarantees convergence because
        // every catalog entry is itself bounded by the exploration horizon.
        let (proposal_batch, proposal_catalog, proposal_blueprints, proposal_feedback) = loop {
            let suffix_ticks = campaign
                .route_tape
                .frames
                .len()
                .saturating_sub(source_frame as usize) as u64;
            let action_catalog_started = Instant::now();
            let proposal_feedback =
                parameterized_feedback_for_state(&campaign, &campaign.current.snapshot, encoder)?;
            let proposals = parameterized_catalog_for_state_with_promoted(
                seed,
                campaign.decision_index,
                &campaign.current.snapshot,
                encoder,
                u32::try_from(maximum_tactic_ticks).map_err(route_error)?,
                proposal_feedback,
                action_schema_sha256,
                &active_tactics,
            )?;
            let (proposals, forced_incumbent_descriptors) = if active_incumbent_continuation
                .is_some_and(ActiveIncumbentContinuation::should_execute_rejoin)
            {
                let targets = incumbent_rejoin_targets(
                    &campaign,
                    active_incumbent_continuation.expect("checked incumbent continuation"),
                )?;
                let (proposals, descriptors) =
                    with_experience_incumbent_rejoins(proposals, &targets)?;
                (proposals, descriptors)
            } else if active_incumbent_continuation
                .is_some_and(ActiveIncumbentContinuation::should_execute_suffix)
            {
                let tape = incumbent_continuation_suffix(
                    &campaign,
                    active_incumbent_continuation.expect("checked incumbent continuation"),
                    encoder,
                )?
                .ok_or_else(|| route_message("incumbent continuation suffix disappeared"))?;
                let (proposals, descriptor) = with_experience_terminal_continuation(
                    proposals,
                    &campaign.current.snapshot,
                    tape,
                )?;
                (proposals, vec![descriptor])
            } else {
                (proposals, Vec::new())
            };
            record_orchestration_detail(
                &mut timing,
                OrchestrationPhase::ActionCatalogConstruction,
                elapsed_micros(action_catalog_started.elapsed()),
            )?;
            let proposal_catalog = Arc::new(proposals.catalog);
            let proposal_blueprints = Arc::new(proposals.blueprints);
            let selection_started = Instant::now();
            let mut preview = online
                .select_action_batch(
                    &campaign,
                    &proposal_catalog,
                    &proposal_blueprints,
                    &encode,
                    TacticQOnlineActionSelectionRequest {
                        family_schema_sha256: action_schema_sha256,
                        maximum_proposals: config.execution_plan.proposal_width_per_decision,
                        acquisition_partition: active_acquisition_rank,
                        policy: config.execution_plan.proposal_policy,
                        goal_distance_feature: Some(encoder.goal_distance_feature()),
                        force_exploration: demonstration_intervention_pending
                            && config.execution_plan.proposal_policy
                                != TacticProposalPolicy::RandomValid,
                    },
                )
                .map_err(route_error)?;
            if !forced_incumbent_descriptors.is_empty() {
                force_incumbent_candidates(&mut preview, &forced_incumbent_descriptors)?;
            }
            let selection_micros = elapsed_micros(selection_started.elapsed());
            timing.tactic_selection_micros = timing
                .tactic_selection_micros
                .saturating_add(selection_micros);
            record_orchestration_detail(
                &mut timing,
                OrchestrationPhase::TacticSelection,
                selection_micros,
            )?;
            if active_paired_terminal_return.is_none()
                && let Some(session) = replay_session.as_mut()
            {
                let learner_refresh_started = Instant::now();
                if let Some(snapshot) = session.pending_snapshot()? {
                    policy_update_probes.push(consume_policy_update_with_probe(
                        session,
                        &mut campaign,
                        &consumed_learner_snapshot,
                        &snapshot,
                        Some(&preview),
                        PolicyUpdateProbeContext {
                            catalog: &proposal_catalog,
                            blueprints: &proposal_blueprints,
                            action_schema_sha256,
                            encoder,
                            maximum_proposals: config.execution_plan.proposal_width_per_decision,
                            acquisition_partition: active_acquisition_rank,
                            proposal_policy: config.execution_plan.proposal_policy,
                            force_exploration: demonstration_intervention_pending
                                && config.execution_plan.proposal_policy
                                    != TacticProposalPolicy::RandomValid,
                        },
                    )?);
                    consumed_learner_snapshot = snapshot;
                    record_orchestration_detail(
                        &mut timing,
                        OrchestrationPhase::LearnerRefresh,
                        elapsed_micros(learner_refresh_started.elapsed()),
                    )?;
                    continue;
                }
                record_orchestration_detail(
                    &mut timing,
                    OrchestrationPhase::LearnerRefresh,
                    elapsed_micros(learner_refresh_started.elapsed()),
                )?;
            }
            let selected_maximum_ticks =
                match plan_online_horizon(preview, suffix_ticks, horizon).map_err(route_error)? {
                    TacticQOnlineHorizonPlan::Execute(batch) => {
                        break (
                            batch,
                            proposal_catalog,
                            proposal_blueprints,
                            proposal_feedback,
                        );
                    }
                    TacticQOnlineHorizonPlan::RestoreCheckpoint {
                        selected_maximum_ticks,
                    } => selected_maximum_ticks,
                };
            episode = episode
                .checked_add(1)
                .ok_or_else(|| route_message("episode counter overflowed"))?;
            let maximum_frontier_frames = usize::try_from(
                source_frame
                    .saturating_add(horizon.saturating_sub(u64::from(selected_maximum_ticks))),
            )
            .map_err(route_error)?;
            let graph_scheduling_started = Instant::now();
            let preferred_restoration_targets = cached_frontiers.pending_locality_targets();
            let continuation = online
                .continue_rollout(
                    &mut campaign,
                    TacticQOnlineRolloutRequest {
                        force_branch: true,
                        active_acquisition_rank,
                        next_acquisition_rank: lane.acquisition.rank(episode),
                        current_rollout_ticks: active_episode_ticks,
                        post_terminal_discovery_tick_budget,
                        demonstration_coverage_pending,
                        terminal_refinement_in_progress: false,
                        terminal_refinement_completed: false,
                        root_refresh_due: lane
                            .root_refresh_due(episode, config.execution_plan.root_refresh_cadence),
                        goal_relabeling_enabled: config
                            .execution_plan
                            .value_treatment
                            .uses_goal_relabeling(),
                        terminal_frontier_action_value_enabled: config
                            .execution_plan
                            .value_treatment
                            .uses_terminal_frontier_action_value(),
                        seed,
                        round: frontier_sampling_round(episode),
                        episode_group: lane.episode_group(episode)?,
                        maximum_route_frames: maximum_frontier_frames,
                        goal_distance_feature: encoder.goal_distance_feature(),
                    },
                    registry,
                    &[],
                    &preferred_restoration_targets,
                    &encode,
                    &|campaign, state| {
                        parameterized_action_surface_for_state(
                            campaign,
                            registry,
                            seed,
                            campaign.decision_index,
                            state,
                            encoder,
                            u32::try_from(maximum_tactic_ticks).map_err(|error| {
                                TacticQCampaignError::Frontier(error.to_string())
                            })?,
                            action_schema_sha256,
                            &active_tactics,
                        )
                        .map_err(|error| TacticQCampaignError::Frontier(error.to_string()))
                    },
                )
                .map_err(route_error)?
                .ok_or_else(|| route_message("forced horizon branch was not scheduled"))?;
            let branch_acquisition_context = continuation.continuation;
            active_acquisition_rank = branch_acquisition_context.acquisition_rank;
            let demonstration_branch = branch_acquisition_context.demonstration;
            let selected_branch = continuation.branch;
            active_incumbent_continuation =
                incumbent_continuation_for_branch(&campaign, &selected_branch)?;
            active_episode_ticks = 0;
            record_orchestration_detail(
                &mut timing,
                OrchestrationPhase::GraphSchedulingAndLeasing,
                elapsed_micros(graph_scheduling_started.elapsed()),
            )?;
            let selected_uncovered_demonstration_frontier = demonstration_branch
                && demonstration_frontier_states
                    .contains(&selected_branch.logical_frontier.state_sha256)
                && !covered_demonstration_frontiers
                    .contains(&selected_branch.logical_frontier.state_sha256);
            demonstration_intervention_pending = first_demonstration_intervention(
                demonstration_coverage_pending,
                branch_acquisition_context.prefer_root,
                selected_uncovered_demonstration_frontier,
            );
            branch_acquisition = selected_branch.acquisition.clone();
            active_terminal_refinement = branch_acquisition.as_ref().and_then(|acquisition| {
                ActiveTerminalRefinementRollout::new(
                    acquisition.replayed_prefix_ticks,
                    acquisition.exact_terminal_ticks_to_go,
                )
            });
            let branch_micros = elapsed_micros(graph_scheduling_started.elapsed());
            timing.checkpoint_branching_micros = timing
                .checkpoint_branching_micros
                .saturating_add(branch_micros);
        };
        demonstration_intervention_pending = false;
        let graph_leasing_started = Instant::now();
        let suffix_ticks = campaign
            .route_tape
            .frames
            .len()
            .saturating_sub(source_frame as usize) as u64;
        let source_snapshot = campaign.current.snapshot.clone();
        let source_snapshot_sha256 = source_snapshot.content_sha256().map_err(route_error)?;
        let source_route_tape = campaign.route_tape.clone();
        let restoration = campaign
            .current_restoration_contract()
            .map_err(route_error)?;
        let paired_terminal_return_seed = if should_start_paired_terminal_return(
            config.execution_plan.paired_terminal_return_evaluation,
            active_paired_terminal_return.is_some(),
        ) {
            PairedTerminalReturnSeed::from_pre_execution_proposals(
                execution_plan_sha256,
                campaign.decision_index,
                restoration.plan.route.route_checkpoint_sha256,
                source_snapshot_sha256,
                branch_acquisition.as_ref(),
                consumed_learner_snapshot.sha256,
                consumed_learner_snapshot.replay_revision,
                &proposal_batch.proposals,
            )
        } else {
            None
        };
        let causal_policy_evaluation =
            active_paired_terminal_return.is_some() || paired_terminal_return_seed.is_some();
        let lease_mode = if causal_policy_evaluation {
            TacticQOnlineLeaseMode::PolicyEvaluation {
                proposal_policy: config.execution_plan.proposal_policy,
            }
        } else if active_incumbent_continuation.is_some_and(|continuation| {
            continuation.should_execute_rejoin() || continuation.should_execute_suffix()
        }) {
            TacticQOnlineLeaseMode::CommittedExploration
        } else {
            TacticQOnlineLeaseMode::Exploration
        };
        let online_lease = match online
            .prepare_decision(
                &mut campaign,
                proposal_batch,
                TacticQOnlineDecisionRequest {
                    suffix_ticks,
                    horizon,
                    maximum_proposals: config.execution_plan.proposal_width_per_decision,
                    learner_model_sha256: consumed_learner_snapshot.sha256,
                    lease_mode,
                },
            )
            .map_err(route_error)?
        {
            TacticQOnlineDecisionPlan::Execute(lease) => lease,
            TacticQOnlineDecisionPlan::RestoreCheckpoint { .. } => {
                return Err(route_message(
                    "online decision no longer fits the horizon after its branch was fixed",
                ));
            }
        };
        let proposal_batch = online_lease.batch;
        let proposal_leases = online_lease.leases;
        let scheduler_decision = online_lease.scheduler_decision;
        let policy_evaluation_decision = online_lease.policy_evaluation_decision;
        let graph_scheduling_timing = online_lease.timing;
        if let Some(breakdown) = timing.orchestration_breakdown.as_mut() {
            breakdown.graph_scheduling_breakdown = breakdown
                .graph_scheduling_breakdown
                .checked_merge(graph_scheduling_timing)
                .ok_or_else(|| route_message("graph scheduling timing overflowed"))?;
        }
        let lease_batch_sha256 = lease_ledger.issue(
            execution_plan_sha256,
            campaign.decision_index,
            &proposal_leases,
        )?;
        record_orchestration_detail(
            &mut timing,
            OrchestrationPhase::GraphSchedulingAndLeasing,
            elapsed_micros(graph_leasing_started.elapsed()),
        )?;

        let decision_index = campaign.decision_index;
        inject_tactic_fault(
            config,
            NativeTacticFaultPoint::BeforeDispatch,
            execution_plan_sha256,
            seed,
            decision_index,
            &seed_root,
        )?;
        let decision_episode_group = campaign.episode_group;
        let learner_snapshot_sha256 = consumed_learner_snapshot.sha256;
        let replay_rows_at_decision = campaign.training_replay_len() as u64;
        let acquisition_rank = active_acquisition_rank;
        let paths_root = seed_root
            .join("native")
            .join(format!("decision-{decision_index:06}"));
        fs::create_dir_all(&paths_root).map_err(route_error)?;
        let freeze_policy_for_paired_return = paired_terminal_return_seed.is_some()
            || active_paired_terminal_return
                .as_ref()
                .is_some_and(ActivePairedTerminalReturn::freezes_policy);
        let source_tape_persistence_started = Instant::now();
        let source_route_tape_ref = decision_content_store
            .store_tape(&source_route_tape)
            .map_err(route_error)?;
        let source_tape_persistence_micros =
            elapsed_micros(source_tape_persistence_started.elapsed());
        record_persistence_timing(
            &mut timing,
            NativeTacticPersistenceTiming {
                source_tape_micros: source_tape_persistence_micros,
                ..NativeTacticPersistenceTiming::default()
            },
        )?;
        let matching_cached_frontier = cached_frontiers
            .matching(
                source_snapshot_sha256,
                source_route_tape.frames.len(),
                restoration.plan.route.route_checkpoint_sha256,
                restoration.plan.route.tape_sha256,
            )
            .cloned();
        let checkpoint_owner_worker_slot = matching_cached_frontier
            .as_ref()
            .map(|frontier| frontier.worker_slot);
        let usable_cached_frontier = matching_cached_frontier
            .as_ref()
            .filter(|frontier| pool.direct_frontier_eligible(frontier));
        let mut directly_restored_frontier = usable_cached_frontier.is_some();
        let mut restore_source = if directly_restored_frontier {
            NativeTacticRestoreSource::ProcessLocalCheckpoint
        } else if source_route_tape.frames.len() > pool.root_source_frame {
            NativeTacticRestoreSource::AuthenticatedRootReplay
        } else {
            NativeTacticRestoreSource::AuthenticatedRoot
        };
        let execution_started = Instant::now();
        let proposal_work = match pool.execute_batch_with_dispatch_hook(
            &proposal_batch.proposals,
            Arc::clone(&proposal_catalog),
            Arc::clone(&proposal_blueprints),
            &source_snapshot,
            &source_route_tape,
            Some(&restoration),
            usable_cached_frontier,
            true,
            &paths_root,
            || {
                inject_tactic_fault(
                    config,
                    NativeTacticFaultPoint::DuringExecution,
                    execution_plan_sha256,
                    seed,
                    decision_index,
                    &seed_root,
                )
            },
        ) {
            Ok(work) => work,
            Err(error) => {
                lease_ledger.resolve(
                    lease_batch_sha256,
                    if error.is_cancelled() {
                        NativeTacticLeaseOutcome::Cancelled
                    } else {
                        NativeTacticLeaseOutcome::Retryable
                    },
                )?;
                return Err(error);
            }
        };
        if let Some(frontier) = usable_cached_frontier {
            let direct_restore_fell_back = proposal_work
                .iter()
                .any(|work| work.restore_accounting.direct_restore_fallback_replays > 0);
            if direct_restore_fell_back {
                cached_frontiers.remove(frontier.worker_slot, &frontier.source.restore_identity);
                directly_restored_frontier = false;
                restore_source = NativeTacticRestoreSource::AuthenticatedRootReplay;
            } else {
                cached_frontiers.touch(frontier.worker_slot, &frontier.source.restore_identity);
                if frontier.source.storage == NativeTacticCheckpointStorage::PortableImage {
                    cached_frontiers.consume_locality_reuse(
                        frontier.worker_slot,
                        &frontier.source.restore_identity,
                    );
                }
            }
        }
        for work in &proposal_work {
            if let Some(source) = work.materialized_checkpoint_source.as_ref() {
                cached_frontiers.retain(CachedTacticFrontier {
                    worker_slot: work.worker_slot,
                    source: source.clone(),
                    state_sha256: source_snapshot_sha256,
                    route_frames: source_route_tape.frames.len(),
                    route_checkpoint_sha256: restoration.plan.route.route_checkpoint_sha256,
                    route_tape_sha256: restoration.plan.route.tape_sha256,
                });
            }
        }
        inject_tactic_fault(
            config,
            NativeTacticFaultPoint::AfterNativeCompletion,
            execution_plan_sha256,
            seed,
            decision_index,
            &seed_root,
        )?;
        let execution_elapsed = execution_started.elapsed();
        let post_execution_orchestration_started = Instant::now();
        if proposal_work
            .iter()
            .any(|work| work.execution_plan_sha256 != execution_plan_sha256)
        {
            lease_ledger.resolve(lease_batch_sha256, NativeTacticLeaseOutcome::Failed)?;
            return Err(route_message(
                "native tactic proposal result is detached from its execution plan",
            ));
        }
        let native_elapsed = proposal_work.iter().fold(Duration::ZERO, |total, work| {
            total.saturating_add(work.native_elapsed)
        });
        let ipc_elapsed = proposal_work.iter().fold(Duration::ZERO, |total, work| {
            total.saturating_add(work.ipc_elapsed)
        });
        let observation_capture_elapsed =
            proposal_work.iter().fold(Duration::ZERO, |total, work| {
                total.saturating_add(work.observation_capture_elapsed)
            });
        let corpus_encoding_elapsed = proposal_work.iter().fold(Duration::ZERO, |total, work| {
            total.saturating_add(work.corpus_encoding_elapsed)
        });
        let rust_state_extraction_micros = proposal_work.iter().fold(0_u64, |total, work| {
            total.saturating_add(work.outcome.state_extraction_micros)
        });
        let preparation_elapsed = proposal_work.iter().fold(Duration::ZERO, |total, work| {
            total.saturating_add(work.preparation_elapsed)
        });
        let mut decision_restore_accounting = NativeTacticRestoreAccounting::default();
        for work in &proposal_work {
            decision_restore_accounting.merge(&work.restore_accounting);
        }
        let proposal_worker_slots = proposal_work
            .iter()
            .map(|work| work.worker_slot)
            .collect::<Vec<_>>();
        let evaluated = proposal_work
            .into_iter()
            .map(|work| {
                campaign
                    .evaluate_rewarded_outcome(work.outcome, &encode, reward_spec)
                    .map_err(route_error)
            })
            .collect::<Result<Vec<_>, _>>();
        let evaluated = match evaluated {
            Ok(evaluated) => evaluated,
            Err(error) => {
                lease_ledger.resolve(lease_batch_sha256, NativeTacticLeaseOutcome::Failed)?;
                return Err(error);
            }
        };
        let evaluated_episode_groups = evaluated
            .iter()
            .enumerate()
            .map(|(proposal_index, _)| {
                if proposal_index == 0 {
                    Ok(decision_episode_group)
                } else {
                    lane.counterfactual_episode_group(
                        decision_index,
                        proposal_index,
                        config.execution_plan.budgets.decisions_per_lane,
                        config.execution_plan.proposal_width_per_decision,
                    )
                }
            })
            .collect::<Result<Vec<_>, NativeTacticRouteRunError>>()?;
        timing.tactic_execution_micros = timing
            .tactic_execution_micros
            .saturating_add(elapsed_micros(execution_elapsed));
        timing.native_simulation_micros = timing
            .native_simulation_micros
            .saturating_add(elapsed_micros(native_elapsed));
        timing.ipc_and_result_transport_micros = timing
            .ipc_and_result_transport_micros
            .saturating_add(elapsed_micros(ipc_elapsed));
        timing.native_observation_capture_micros = timing
            .native_observation_capture_micros
            .saturating_add(elapsed_micros(observation_capture_elapsed));
        timing.native_corpus_encoding_micros = timing
            .native_corpus_encoding_micros
            .saturating_add(elapsed_micros(corpus_encoding_elapsed));
        timing.rust_state_extraction_micros = timing
            .rust_state_extraction_micros
            .saturating_add(rust_state_extraction_micros);
        timing.tactic_preparation_and_fact_extraction_micros = timing
            .tactic_preparation_and_fact_extraction_micros
            .saturating_add(elapsed_micros(preparation_elapsed));
        let result_validation_micros =
            elapsed_micros(post_execution_orchestration_started.elapsed());
        timing.result_validation_and_fact_extraction_micros = timing
            .result_validation_and_fact_extraction_micros
            .saturating_add(result_validation_micros);
        record_orchestration_detail(
            &mut timing,
            OrchestrationPhase::ResultValidationAndFactExtraction,
            result_validation_micros,
        )?;
        let admission_orchestration_started = Instant::now();
        // The policy-selected proposal is authoritative. Sibling proposals are
        // native training and candidate evidence, not an outcome-peeking beam
        // search that replaces the learner's action after observing results.
        let winner_index = 0;
        let winning_outcome = evaluated[winner_index].outcome.clone();
        let next_action_catalog_started = Instant::now();
        let next_proposals = parameterized_catalog_for_state_with_promoted(
            seed,
            campaign
                .decision_index
                .checked_add(1)
                .ok_or_else(|| route_message("decision index overflowed"))?,
            &winning_outcome.next_facts,
            encoder,
            u32::try_from(maximum_tactic_ticks).map_err(route_error)?,
            None,
            action_schema_sha256,
            &active_tactics,
        )?;
        let next_action_catalog_micros = elapsed_micros(next_action_catalog_started.elapsed());
        record_orchestration_detail(
            &mut timing,
            OrchestrationPhase::ActionCatalogConstruction,
            next_action_catalog_micros,
        )?;
        let policy_update = if config.execution_plan.proposal_policy
            == TacticProposalPolicy::FrozenPolicy
            || freeze_policy_for_paired_return
        {
            TacticQOnlinePolicyUpdate::Frozen
        } else {
            TacticQOnlinePolicyUpdate::Adaptive { refit_model: false }
        };
        let online_admission = match online.admit(
            &mut campaign,
            &proposal_batch,
            &evaluated,
            &evaluated_episode_groups,
            &proposal_leases,
            &next_proposals.catalog,
            &next_proposals.blueprints,
            registry,
            &encode,
            |_| true,
            reward_spec,
            policy_update,
        ) {
            Ok(admission) => admission,
            Err(error) => {
                lease_ledger.resolve(lease_batch_sha256, NativeTacticLeaseOutcome::Failed)?;
                return Err(error.into());
            }
        };
        if let Some(continuation) = active_incumbent_continuation.as_mut() {
            continuation
                .record_executed_action(
                    &winning_outcome.selected.descriptor.option_id,
                    winning_outcome.execution.end_reason == OptionEndReason::Terminated,
                )
                .map_err(route_message)?;
        }
        let terminal_candidates = online_admission.terminal_candidates;
        let newly_admitted_training_rows = online_admission.newly_admitted_training_rows as u64;
        let duplicate_training_transitions = online_admission.duplicate_training_transitions as u64;
        let evaluated_native_ticks = online_admission.evaluated_native_ticks;
        let best_authenticated_terminal_ticks = online_admission.best_authenticated_terminal_ticks;
        let terminal_projection_micros = online_admission.timing.terminal_projection_micros;
        let batch_graph_admission_micros = online_admission.timing.graph_admission_micros;
        let selected_outcome_retention_micros =
            online_admission.timing.selected_outcome_retention_micros;
        let step = online_admission.step;
        active_episode_ticks = active_episode_ticks
            .checked_add(u64::from(winning_outcome.execution.duration.realized_ticks))
            .ok_or_else(|| route_message("active episode tick count overflowed"))?;
        timing.graph_admission_micros = timing
            .graph_admission_micros
            .saturating_add(selected_outcome_retention_micros);
        let frontier_retention_started = Instant::now();
        let retained_restoration = (!winning_outcome.terminal)
            .then(|| campaign.current_restoration_contract().map_err(route_error))
            .transpose()?;
        let retained_frontier = match (
            winning_outcome.retained_native_checkpoint.as_ref(),
            winning_outcome
                .retained_native_boundary_fingerprint
                .as_ref(),
            retained_restoration.as_ref(),
        ) {
            (Some(checkpoint), Some(boundary_fingerprint), Some(restoration))
                if checkpoint.route_ticks
                    == winning_outcome
                        .route_tape
                        .frames
                        .len()
                        .saturating_sub(source_frame as usize) as u64 =>
            {
                Some(CachedTacticFrontier {
                    worker_slot: proposal_worker_slots[winner_index],
                    source: NativeTacticCheckpointSource {
                        restore_identity: checkpoint.restore_identity.clone(),
                        boundary_fingerprint: boundary_fingerprint.clone(),
                        route_ticks: checkpoint.route_ticks as usize,
                        storage: match checkpoint.storage_kind.as_str() {
                            "portable_image" => NativeTacticCheckpointStorage::PortableImage,
                            "live_endpoint" => NativeTacticCheckpointStorage::LiveEndpoint,
                            _ => {
                                return Err(route_message(
                                    "retained native checkpoint has an unknown storage kind",
                                ));
                            }
                        },
                    },
                    state_sha256: step.step.transition.after_state_sha256,
                    route_frames: winning_outcome.route_tape.frames.len(),
                    route_checkpoint_sha256: restoration.plan.route.route_checkpoint_sha256,
                    route_tape_sha256: restoration.plan.route.tape_sha256,
                })
            }
            (None, None, _) | (_, _, None) => None,
            _ => {
                return Err(route_message(
                    "retained native checkpoint lacks its exact endpoint boundary",
                ));
            }
        };
        if let Some(frontier) = retained_frontier {
            cached_frontiers.retain(frontier);
        }
        let frontier_retention_micros = elapsed_micros(frontier_retention_started.elapsed());
        let campaign_admission_micros = elapsed_micros(admission_orchestration_started.elapsed());
        let mut admission_breakdown = NativeTacticCampaignAdmissionTiming {
            terminal_projection_micros,
            batch_graph_admission_micros,
            next_action_catalog_micros,
            selected_outcome_retention_micros,
            frontier_retention_micros,
            unattributed_micros: 0,
        };
        admission_breakdown.unattributed_micros =
            campaign_admission_micros.saturating_sub(admission_breakdown.total_micros());
        timing
            .campaign_admission_breakdown
            .get_or_insert_default()
            .merge(admission_breakdown);
        timing.campaign_admission_micros = timing
            .campaign_admission_micros
            .saturating_add(campaign_admission_micros);
        record_orchestration_detail(
            &mut timing,
            OrchestrationPhase::CampaignAdmission,
            campaign_admission_micros
                .checked_sub(next_action_catalog_micros)
                .ok_or_else(|| {
                    route_message("native tactic campaign admission timing is detached")
                })?,
        )?;
        let evidence_started = Instant::now();
        let selected = &step.step.decision.selected;
        *selection_counts
            .entry(selected.descriptor.option_id.clone())
            .or_default() += 1;
        let selected_q = step
            .step
            .decision
            .ranking
            .values
            .ranked
            .iter()
            .find(|ranked| ranked.descriptor == selected.descriptor)
            .map(|ranked| ranked.mean_q);
        let best_q = step
            .step
            .decision
            .ranking
            .values
            .ranked
            .first()
            .map(|ranked| ranked.mean_q);
        let action_surface = step
            .step
            .decision
            .ranking
            .choices
            .iter()
            .map(|choice| {
                let estimate = step
                    .step
                    .decision
                    .ranking
                    .values
                    .ranked
                    .iter()
                    .find(|ranked| ranked.descriptor == choice.descriptor);
                NativeTacticValueTrace {
                    option_id: choice.choice_id.clone(),
                    descriptor: Some(choice.descriptor.clone()),
                    applicable: choice.applicable,
                    mean_q: estimate.map(|ranked| ranked.mean_q),
                    ensemble_variance: estimate.map(|ranked| ranked.ensemble_variance),
                    selected: choice.choice_id == selected.descriptor.option_id,
                }
            })
            .collect::<Vec<_>>();
        if action_surface.is_empty()
            || action_surface
                .iter()
                .filter(|tactic| tactic.selected)
                .count()
                != 1
            || action_surface
                .iter()
                .any(|tactic| tactic.selected && !tactic.applicable)
        {
            return Err(route_message(
                "selected tactic is detached from the applicable action surface",
            ));
        }
        native_ticks = native_ticks.saturating_add(evaluated_native_ticks);
        let frontier_cells = campaign.frontier_cell_count();
        let before_features = encoder
            .encode(&step.step.transition.before)
            .map_err(route_error)?;
        let after_features = encoder
            .encode(&step.step.transition.after)
            .map_err(route_error)?;
        let proposal_traces = evaluated
            .iter()
            .enumerate()
            .map(|(index, proposal)| {
                let after_features = encoder
                    .encode(&proposal.transition.after)
                    .map_err(route_error)?;
                let reachability = proposal_batch
                    .goal_reachability_estimates
                    .iter()
                    .find(|estimate| estimate.descriptor == proposal.outcome.selected.descriptor);
                Ok(NativeTacticProposalTrace {
                    execution_plan_sha256,
                    option_id: proposal.outcome.selected.descriptor.option_id.clone(),
                    selection_reason: proposal.outcome.selected.reason,
                    predicted_goal_progress_per_tick: reachability
                        .map(|estimate| estimate.predicted_goal_progress_per_tick),
                    reachability_nearest_distance: reachability
                        .map(|estimate| estimate.nearest_distance),
                    reward: proposal.reward.training_reward,
                    reward_components: proposal.reward.clone(),
                    realized_ticks: proposal.outcome.execution.duration.realized_ticks,
                    root_route_ticks: proposal.outcome.route_tape.frames.len() as u64,
                    emitted_tape_sha256: proposal.transition.value_sample.realized_tape_sha256,
                    terminal: proposal.outcome.terminal,
                    goal_distance_after: after_features[encoder.goal_distance_feature()],
                    after_snapshot_sha256: proposal.transition.after_state_sha256,
                    after_checkpoint_sha256: proposal.transition.next_checkpoint_sha256,
                    retained: index == winner_index,
                })
            })
            .collect::<Result<Vec<_>, NativeTacticRouteRunError>>()?;
        let paired_terminal_return = if let Some(pair_seed) = paired_terminal_return_seed {
            let control = evaluated
                .get(pair_seed.control_proposal_index())
                .ok_or_else(|| route_message("paired terminal-return control disappeared"))?;
            let policy_route_ticks = u64::try_from(winning_outcome.route_tape.frames.len())
                .map_err(route_error)?
                .saturating_sub(source_frame);
            let control_route_ticks = u64::try_from(control.outcome.route_tape.frames.len())
                .map_err(route_error)?
                .saturating_sub(source_frame);
            let (active, trace) = pair_seed.admit_first_steps(
                winning_outcome.terminal,
                policy_route_ticks,
                control.transition.next_checkpoint_sha256,
                control.transition.after_state_sha256,
                control.outcome.execution.duration.realized_ticks,
                control.outcome.terminal,
                control_route_ticks,
            )?;
            active_paired_terminal_return = active;
            active_terminal_refinement = active_paired_terminal_return
                .as_ref()
                .map(ActivePairedTerminalReturn::rollout);
            Some(trace)
        } else if let Some(pair) = active_paired_terminal_return.as_mut() {
            if learner_snapshot_sha256 != pair.frozen_learner_snapshot_sha256() {
                return Err(route_message(
                    "paired terminal-return continuation changed learner authority",
                ));
            }
            let current_route_ticks = u64::try_from(campaign.route_tape.frames.len())
                .map_err(route_error)?
                .saturating_sub(source_frame);
            let trace = pair.record_decision(winning_outcome.terminal, current_route_ticks);
            if pair.complete() {
                active_paired_terminal_return = None;
                active_terminal_refinement = None;
            } else {
                active_terminal_refinement = active_paired_terminal_return
                    .as_ref()
                    .map(ActivePairedTerminalReturn::rollout);
            }
            Some(trace)
        } else {
            None
        };
        decision_restore_accounting.proposal_transitions = proposal_traces.len() as u64;
        decision_restore_accounting.useful_transitions = proposal_traces
            .iter()
            .filter(|proposal| {
                proposal.terminal
                    || proposal.reward > 0.0
                    || proposal.goal_distance_after
                        < before_features[encoder.goal_distance_feature()]
            })
            .count() as u64;
        decision_restore_accounting.refresh_rates();
        native_restore_accounting.merge(&decision_restore_accounting);
        let best_authenticated_tick_after_decision =
            best_authenticated_terminal_ticks.and_then(|ticks| ticks.checked_sub(1));
        let decision_trace = NativeTacticDecisionTrace {
            execution_plan_sha256,
            decision_index,
            cumulative_wall_micros: prior_wall_micros
                .saturating_add(elapsed_micros(invocation_started.elapsed())),
            learner_snapshot_sha256,
            replay_rows_at_decision,
            replay_generation: lane.generation_index as u64,
            lane_index: lane.lane_index,
            lane_role: Some(lane.role),
            acquisition_rank,
            policy_update_probes,
            frontier_identity: source_snapshot_sha256,
            checkpoint_owner_worker_slot,
            proposal_worker_slots,
            restore_source: Some(restore_source),
            result_admission_schema: NATIVE_TACTIC_RESULT_ADMISSION_SCHEMA_V1.into(),
            episode,
            source_route_ticks: source_route_tape.frames.len() as u64,
            route_suffix_ticks: campaign
                .route_tape
                .frames
                .len()
                .saturating_sub(source_frame as usize) as u64,
            selected_option_id: selected.descriptor.option_id.clone(),
            selection_reason: selected.reason,
            selected_q,
            best_q,
            reward: step.reward.training_reward,
            reward_components: step.reward.clone(),
            goal_distance_before: before_features[encoder.goal_distance_feature()],
            goal_distance_after: after_features[encoder.goal_distance_feature()],
            terminal: step.step.transition.value_sample.terminal,
            newly_admitted_training_rows,
            duplicate_training_transitions,
            training_replay_rows: campaign.training_replay_len() as u64,
            scheduler_decision,
            policy_evaluation_decision,
            branch_acquisition: branch_acquisition.take(),
            paired_terminal_return,
            frontier_cells,
            logical_frontier_records: frontier_cells.saturating_add(1),
            directly_restorable_native_frontiers: usize::from(directly_restored_frontier),
            replay_only_frontiers: frontier_cells
                .saturating_sub(usize::from(directly_restored_frontier)),
            visited_states: campaign.visited_state_count(),
            completed_executable_graph_expansions: u64::try_from(
                campaign.completed_executable_graph_expansion_count(),
            )
            .map_err(route_error)?,
            best_authenticated_tick_after_decision,
            before: tactic_state_trace(&step.step.transition.before)?,
            after: tactic_state_trace(&step.step.transition.after)?,
            measurements: Vec::new(),
            applicable_tactics: action_surface,
            proposal_feedback,
            proposal_batch: proposal_traces,
            goal_reachability_calibration: proposal_batch.goal_reachability_calibration.clone(),
            terminal_action_calibration: proposal_batch.terminal_action_calibration.clone(),
        };
        if decision_trace_is_useful(&decision_trace) {
            useful_decisions = useful_decisions.saturating_add(1);
        }
        let proposal_components = retained_replay_components(
            &evaluated,
            &decision_trace.proposal_batch,
            &proposal_catalog,
            before_features[encoder.goal_distance_feature()],
        )?;
        let evidence_micros = elapsed_micros(evidence_started.elapsed());
        timing.evidence_projection_micros = timing
            .evidence_projection_micros
            .saturating_add(evidence_micros);
        timing.evidence_projection_and_persistence_micros = timing
            .evidence_projection_and_persistence_micros
            .saturating_add(evidence_micros);
        let persistence_started = Instant::now();
        let model_update_micros_before = timing.model_update_micros;
        let replay_content_started = Instant::now();
        let replay_content = persist_evaluated_replay_content(&decision_content_store, &evaluated)?;
        let replay_content_micros = elapsed_micros(replay_content_started.elapsed());
        let proposal_records = decision_trace
            .proposal_batch
            .iter()
            .zip(proposal_components)
            .zip(&replay_content)
            .map(|((trace, component), content)| NativeTacticProposalRecord {
                trace: trace.clone(),
                component,
                transition: Some(content.transition),
                inline_transition: None,
            })
            .collect::<Vec<_>>();
        let transition_ref = replay_content[winner_index].transition;
        timing.wall_micros =
            prior_wall_micros.saturating_add(elapsed_micros(invocation_started.elapsed()));
        let recovery_decision_index = campaign.decision_index;
        let recovery_checkpoint_started = Instant::now();
        persist_tactic_recovery_point(
            &seed_root,
            &mut campaign,
            &decision_content_store,
            NativeTacticSeedPerformance {
                schema: TACTIC_ROUTE_PERFORMANCE_SCHEMA_V2.into(),
                decisions: recovery_decision_index,
                useful_decisions,
                native_restore_accounting: native_restore_accounting.clone(),
                timing: timing.clone(),
            },
        )?;
        let recovery_checkpoint_micros = elapsed_micros(recovery_checkpoint_started.elapsed());
        inject_tactic_fault(
            config,
            NativeTacticFaultPoint::AfterRecoveryPointCommit,
            execution_plan_sha256,
            seed,
            decision_index,
            &seed_root,
        )?;
        let decision_journal_started = Instant::now();
        decision_journal.append(&decision_record(
            &decision_trace,
            campaign.episode_group,
            campaign.root_checkpoint_sha256,
            root_tape_ref,
            Some(source_route_tape_ref),
            Some(transition_ref),
            None,
            proposal_records,
        ))?;
        let decision_journal_micros = elapsed_micros(decision_journal_started.elapsed());
        inject_tactic_fault(
            config,
            NativeTacticFaultPoint::AfterDecisionCommit,
            execution_plan_sha256,
            seed,
            decision_index,
            &seed_root,
        )?;
        let mut replay_publication_micros = 0;
        if let Some(session) = replay_session.as_mut() {
            let replay_publication_started = Instant::now();
            let publish = session.publish_evaluated(
                decision_index,
                learner_snapshot_sha256,
                &evaluated,
                &evaluated_episode_groups,
            )?;
            let replay_publication_elapsed = elapsed_micros(replay_publication_started.elapsed());
            if publish.admitted_rows.saturating_add(publish.duplicate_rows)
                != evaluated.len() as u64
            {
                return Err(route_message(
                    "campaign learner did not account for every committed proposal",
                ));
            }
            replay_publication_micros =
                replay_publication_elapsed.saturating_sub(publish.update.update_micros);
            timing.model_update_micros = timing
                .model_update_micros
                .saturating_add(publish.update.update_micros);
        }
        let lease_resolution_started = Instant::now();
        lease_ledger.resolve(lease_batch_sha256, NativeTacticLeaseOutcome::Completed)?;
        let lease_resolution_micros = elapsed_micros(lease_resolution_started.elapsed());
        trace.push(decision_trace);
        if demonstration_curriculum
            && demonstration_frontier_states.contains(&source_snapshot_sha256)
        {
            covered_demonstration_frontiers.insert(source_snapshot_sha256);
        }
        let recovery_prune_started = Instant::now();
        prune_tactic_recovery_points(&seed_root, campaign.decision_index)?;
        let recovery_prune_micros = elapsed_micros(recovery_prune_started.elapsed());
        let retained_terminal_started = Instant::now();
        for candidate in terminal_candidates {
            if !campaign
                .final_result_matches_graph_terminal(&candidate)
                .map_err(route_error)?
            {
                continue;
            }
            retain_successful_result(
                &retained_success_root,
                decision_index,
                &candidate,
                &mut best_success,
            )?;
        }
        let retained_terminal_micros = elapsed_micros(retained_terminal_started.elapsed());
        if cancellation_requested(config) {
            return Err(route_cancelled("native tactic route paused"));
        }
        let persistence_elapsed = elapsed_micros(persistence_started.elapsed());
        let model_update_micros = timing
            .model_update_micros
            .saturating_sub(model_update_micros_before);
        let mut persistence_breakdown = NativeTacticPersistenceTiming {
            replay_content_micros,
            recovery_checkpoint_micros,
            decision_journal_micros,
            replay_publication_micros,
            lease_resolution_micros,
            recovery_prune_micros,
            retained_terminal_micros,
            ..NativeTacticPersistenceTiming::default()
        };
        let named_persistence_micros = persistence_breakdown
            .checked_total_micros()
            .ok_or_else(|| route_message("native tactic persistence timing overflowed"))?;
        persistence_breakdown.unattributed_micros = persistence_elapsed
            .checked_sub(model_update_micros)
            .and_then(|without_model| without_model.checked_sub(named_persistence_micros))
            .ok_or_else(|| route_message("native tactic persistence phases exceed wall"))?;
        record_persistence_timing(&mut timing, persistence_breakdown)?;
        let iteration_wall_micros = elapsed_micros(iteration_started.elapsed());
        let iteration_known_top_micros = iteration_top_baseline.checked_delta_total(&timing)?;
        let iteration_orchestration_micros = iteration_wall_micros
            .checked_sub(iteration_known_top_micros)
            .ok_or_else(|| route_message("native tactic decision timing is detached"))?;
        if let Some(detail_before) = iteration_orchestration_detail_baseline {
            let detail_after = orchestration_detail_total(&timing)?.ok_or_else(|| {
                route_message("native tactic orchestration detail disappeared during a decision")
            })?;
            let named_detail_micros = detail_after.checked_sub(detail_before).ok_or_else(|| {
                route_message("native tactic orchestration detail regressed during a decision")
            })?;
            let decision_bookkeeping_micros = iteration_orchestration_micros
                .checked_sub(named_detail_micros)
                .ok_or_else(|| {
                    route_message("native tactic named decision phases exceed decision wall")
                })?;
            record_orchestration_detail(
                &mut timing,
                OrchestrationPhase::DecisionBookkeeping,
                decision_bookkeeping_micros,
            )?;
        }
        record_orchestration_total(&mut timing, iteration_orchestration_micros)?;
    }

    finalize_seed(
        config,
        lane,
        execution_plan_sha256,
        seed,
        &seed_root,
        &retained_success_root,
        &decision_content_store,
        invocation_started,
        prior_wall_micros,
        prior_model_update_micros,
        campaign,
        trace,
        selection_counts,
        native_ticks,
        episode,
        useful_decisions,
        native_restore_accounting,
        timing,
        best_success,
        replay_session,
        lease_ledger,
    )
}
