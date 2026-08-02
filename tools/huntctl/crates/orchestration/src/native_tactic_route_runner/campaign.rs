pub(super) use super::campaign_schedule::{
    first_demonstration_intervention, next_branch_acquisition_rank,
    prefer_root_for_periodic_branch, should_probe_policy_before_branch,
    should_rank_frontier_with_live_model, should_schedule_branch,
};
use super::*;

mod seed_finalization;
use seed_finalization::finalize_seed;
mod seed_initialization;
use seed_initialization::{SeedCampaignInitialization, load_or_create_seed_campaign};

pub(super) const NATIVE_TACTIC_RESULT_ADMISSION_SCHEMA_V1: &str =
    "dusklight-native-tactic-result-admission/v1";

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
    // Process-local handles intentionally do not survive campaign resume.
    let mut cached_frontier: Option<CachedTacticFrontier> = None;
    let mut branch_acquisition: Option<TacticFrontierAcquisition> = None;
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
    let mut replay_session = build_replay_session(
        config.execution_plan,
        live_learner,
        lane,
        inherited_learner_snapshot.replay_revision,
    )?;
    if resuming_seed && let Some(session) = replay_session.as_ref() {
        session.repair_committed(&campaign, &trace)?;
    }
    let mut consumed_learner_snapshot = inherited_learner_snapshot;
    let mut lease_ledger = NativeTacticLeaseLedger::open(&seed_root)?;
    if resuming_seed {
        let completed_expansions_by_decision = trace
            .iter()
            .filter_map(|decision| {
                decision.scheduler_decision.as_ref().map(|scheduler| {
                    (
                        decision.decision_index,
                        scheduler.evaluated_expansion_sha256.clone(),
                    )
                })
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
        let planned_acquisition_rank = lane.acquisition.rank(campaign.decision_index);
        let terminal_restart = campaign.current.snapshot.terminal.reached == Some(true);
        let mut policy_update_probes = Vec::new();
        if should_probe_policy_before_branch(terminal_restart)
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
                    promoted_tactics,
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
                        acquisition_partition: planned_acquisition_rank,
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
        let next_branch_acquisition_rank = next_branch_acquisition_rank(lane.acquisition, episode)
            .ok_or_else(|| route_message("episode counter overflowed"))?;
        let terminal_support_acquisition =
            campaign.native_terminal_supported() && next_branch_acquisition_rank == 0;
        let demonstration_coverage_pending = demonstration_curriculum
            && covered_demonstration_frontiers.len() < demonstration_frontier_states.len();
        let scheduled_branch = should_schedule_branch(
            campaign.decision_index,
            config.execution_plan.branch_every_decisions,
            terminal_restart,
            terminal_support_acquisition,
            demonstration_coverage_pending,
        );
        if !campaign.replay().is_empty() && scheduled_branch {
            let branch_started = Instant::now();
            episode = episode
                .checked_add(1)
                .ok_or_else(|| route_message("episode counter overflowed"))?;
            let maximum_frontier_frames = usize::try_from(
                source_frame.saturating_add(horizon.saturating_sub(maximum_tactic_ticks)),
            )
            .map_err(route_error)?;
            let graph_scheduling_started = Instant::now();
            let demonstration_branch = demonstration_coverage_pending && !terminal_restart;
            let ranked_frontier_branch = should_rank_frontier_with_live_model(
                demonstration_branch,
                terminal_support_acquisition,
                config
                    .execution_plan
                    .value_treatment
                    .uses_terminal_frontier_action_value(),
            );
            let [root, frontier] = if ranked_frontier_branch {
                campaign.sample_root_and_ranked_frontier(
                    seed,
                    frontier_sampling_round(episode),
                    &[],
                    maximum_frontier_frames,
                    demonstration_branch,
                    encoder.goal_distance_feature(),
                    &encode,
                    &|state| {
                        applicable_parameterized_descriptors_for_state(
                            &campaign,
                            registry,
                            seed,
                            campaign.decision_index,
                            state,
                            encoder,
                            u32::try_from(maximum_tactic_ticks).map_err(route_error)?,
                            action_schema_sha256,
                        )
                    },
                )
            } else {
                let acquisition_rank = if terminal_restart || terminal_support_acquisition {
                    0
                } else {
                    lane.acquisition.rank(episode)
                };
                campaign.graph_scheduled_root_and_frontier(
                    seed,
                    frontier_sampling_round(episode),
                    acquisition_rank,
                    maximum_frontier_frames,
                )
            }
            .map_err(route_error)?;
            record_orchestration_detail(
                &mut timing,
                OrchestrationPhase::GraphSchedulingAndLeasing,
                elapsed_micros(graph_scheduling_started.elapsed()),
            )?;
            let prefer_root = prefer_root_for_periodic_branch(
                terminal_restart || (terminal_support_acquisition && !demonstration_branch),
                lane.root_refresh_due(episode, config.execution_plan.root_refresh_cadence),
            );
            let selected_branch = if prefer_root { &root } else { &frontier };
            let selected_uncovered_demonstration_frontier = demonstration_branch
                && demonstration_frontier_states
                    .contains(&selected_branch.logical_frontier.state_sha256)
                && !covered_demonstration_frontiers
                    .contains(&selected_branch.logical_frontier.state_sha256);
            demonstration_intervention_pending = first_demonstration_intervention(
                demonstration_coverage_pending,
                prefer_root,
                selected_uncovered_demonstration_frontier,
            );
            branch_acquisition = selected_branch.acquisition.clone();
            let action_catalog_started = Instant::now();
            let branch_proposals = parameterized_catalog_for_state_with_promoted(
                seed,
                campaign.decision_index,
                &selected_branch.state,
                encoder,
                u32::try_from(maximum_tactic_ticks).map_err(route_error)?,
                parameterized_feedback_for_state(&campaign, &selected_branch.state, encoder)?,
                action_schema_sha256,
                promoted_tactics,
            )?;
            record_orchestration_detail(
                &mut timing,
                OrchestrationPhase::ActionCatalogConstruction,
                elapsed_micros(action_catalog_started.elapsed()),
            )?;
            let branch_restore_started = Instant::now();
            campaign
                .restore_branch(
                    selected_branch,
                    lane.episode_group(episode)?,
                    registry,
                    &branch_proposals.catalog,
                    &branch_proposals.blueprints,
                    |_| true,
                )
                .map_err(route_error)?;
            record_orchestration_detail(
                &mut timing,
                OrchestrationPhase::GraphSchedulingAndLeasing,
                elapsed_micros(branch_restore_started.elapsed()),
            )?;
            let branch_micros = elapsed_micros(branch_started.elapsed());
            timing.checkpoint_branching_micros = timing
                .checkpoint_branching_micros
                .saturating_add(branch_micros);
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
                promoted_tactics,
            )?;
            record_orchestration_detail(
                &mut timing,
                OrchestrationPhase::ActionCatalogConstruction,
                elapsed_micros(action_catalog_started.elapsed()),
            )?;
            let proposal_catalog = Arc::new(proposals.catalog);
            let proposal_blueprints = Arc::new(proposals.blueprints);
            let selection_started = Instant::now();
            let mut preview = campaign
                .decide_parameterized_batch_with_policy(
                    &proposal_catalog,
                    &proposal_blueprints,
                    action_schema_sha256,
                    &encode,
                    config.execution_plan.proposal_width_per_decision,
                    lane.acquisition.rank(campaign.decision_index),
                    config.execution_plan.proposal_policy,
                    Some(encoder.goal_distance_feature()),
                    demonstration_intervention_pending
                        && config.execution_plan.proposal_policy
                            != TacticProposalPolicy::RandomValid,
                )
                .map_err(route_error)?;
            let selection_micros = elapsed_micros(selection_started.elapsed());
            timing.tactic_selection_micros = timing
                .tactic_selection_micros
                .saturating_add(selection_micros);
            record_orchestration_detail(
                &mut timing,
                OrchestrationPhase::TacticSelection,
                selection_micros,
            )?;
            if let Some(session) = replay_session.as_mut() {
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
                            acquisition_partition: planned_acquisition_rank,
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
            let primary = preview
                .proposals
                .first()
                .ok_or_else(|| route_message("tactic proposal batch is empty"))?;
            let selected_maximum_ticks = preview
                .ranking
                .choices
                .iter()
                .find(|choice| choice.choice_id == primary.descriptor.option_id)
                .ok_or_else(|| route_message("selected tactic is absent from its catalog"))?
                .duration
                .maximum_ticks;
            if selected_tactic_fits_horizon(suffix_ticks, selected_maximum_ticks, horizon) {
                preview.proposals.retain(|proposal| {
                    preview
                        .ranking
                        .choices
                        .iter()
                        .find(|choice| choice.choice_id == proposal.descriptor.option_id)
                        .is_some_and(|choice| {
                            selected_tactic_fits_horizon(
                                suffix_ticks,
                                choice.duration.maximum_ticks,
                                horizon,
                            )
                        })
                });
                break (
                    preview,
                    proposal_catalog,
                    proposal_blueprints,
                    proposal_feedback,
                );
            }
            let branch_started = Instant::now();
            episode = episode
                .checked_add(1)
                .ok_or_else(|| route_message("episode counter overflowed"))?;
            let maximum_frontier_frames = usize::try_from(
                source_frame
                    .saturating_add(horizon.saturating_sub(u64::from(selected_maximum_ticks))),
            )
            .map_err(route_error)?;
            let graph_scheduling_started = Instant::now();
            let demonstration_branch = demonstration_coverage_pending && !terminal_restart;
            let ranked_frontier_branch = should_rank_frontier_with_live_model(
                demonstration_branch,
                terminal_support_acquisition,
                config
                    .execution_plan
                    .value_treatment
                    .uses_terminal_frontier_action_value(),
            );
            let [root, frontier] = if ranked_frontier_branch {
                campaign.sample_root_and_ranked_frontier(
                    seed,
                    frontier_sampling_round(episode),
                    &[],
                    maximum_frontier_frames,
                    demonstration_branch,
                    encoder.goal_distance_feature(),
                    &encode,
                    &|state| {
                        applicable_parameterized_descriptors_for_state(
                            &campaign,
                            registry,
                            seed,
                            campaign.decision_index,
                            state,
                            encoder,
                            u32::try_from(maximum_tactic_ticks).map_err(route_error)?,
                            action_schema_sha256,
                        )
                    },
                )
            } else {
                let acquisition_rank = if terminal_restart || terminal_support_acquisition {
                    0
                } else {
                    lane.acquisition.rank(episode)
                };
                campaign.graph_scheduled_root_and_frontier(
                    seed,
                    frontier_sampling_round(episode),
                    acquisition_rank,
                    maximum_frontier_frames,
                )
            }
            .map_err(route_error)?;
            record_orchestration_detail(
                &mut timing,
                OrchestrationPhase::GraphSchedulingAndLeasing,
                elapsed_micros(graph_scheduling_started.elapsed()),
            )?;
            let prefer_root = prefer_root_for_periodic_branch(
                terminal_restart || (terminal_support_acquisition && !demonstration_branch),
                lane.root_refresh_due(episode, config.execution_plan.root_refresh_cadence),
            );
            let selected_branch = if prefer_root { &root } else { &frontier };
            let selected_uncovered_demonstration_frontier = demonstration_branch
                && demonstration_frontier_states
                    .contains(&selected_branch.logical_frontier.state_sha256)
                && !covered_demonstration_frontiers
                    .contains(&selected_branch.logical_frontier.state_sha256);
            demonstration_intervention_pending = first_demonstration_intervention(
                demonstration_coverage_pending,
                prefer_root,
                selected_uncovered_demonstration_frontier,
            );
            branch_acquisition = selected_branch.acquisition.clone();
            let action_catalog_started = Instant::now();
            let branch_proposals = parameterized_catalog_for_state_with_promoted(
                seed,
                campaign.decision_index,
                &selected_branch.state,
                encoder,
                u32::try_from(maximum_tactic_ticks).map_err(route_error)?,
                parameterized_feedback_for_state(&campaign, &selected_branch.state, encoder)?,
                action_schema_sha256,
                promoted_tactics,
            )?;
            record_orchestration_detail(
                &mut timing,
                OrchestrationPhase::ActionCatalogConstruction,
                elapsed_micros(action_catalog_started.elapsed()),
            )?;
            let branch_restore_started = Instant::now();
            campaign
                .restore_branch(
                    selected_branch,
                    lane.episode_group(episode)?,
                    registry,
                    &branch_proposals.catalog,
                    &branch_proposals.blueprints,
                    |_| true,
                )
                .map_err(route_error)?;
            record_orchestration_detail(
                &mut timing,
                OrchestrationPhase::GraphSchedulingAndLeasing,
                elapsed_micros(branch_restore_started.elapsed()),
            )?;
            let branch_micros = elapsed_micros(branch_started.elapsed());
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
        let eligible_descriptors = proposal_batch
            .ranking
            .choices
            .iter()
            .filter(|choice| {
                choice.applicable
                    && selected_tactic_fits_horizon(
                        suffix_ticks,
                        choice.duration.maximum_ticks,
                        horizon,
                    )
            })
            .map(|choice| choice.descriptor.clone())
            .collect::<Vec<_>>();
        let leased_batch = campaign
            .lease_current_parameterized_batch(
                proposal_batch,
                &eligible_descriptors,
                config.execution_plan.proposal_width_per_decision,
                consumed_learner_snapshot.sha256,
            )
            .map_err(route_error)?;
        if let Some(breakdown) = timing.orchestration_breakdown.as_mut() {
            breakdown.graph_scheduling_breakdown = breakdown
                .graph_scheduling_breakdown
                .checked_merge(leased_batch.timing)
                .ok_or_else(|| route_message("graph scheduling timing overflowed"))?;
        }
        let proposal_batch = leased_batch.batch;
        let proposal_leases = leased_batch.leases;
        let scheduler_decision = leased_batch.scheduler_decision;
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
        let acquisition_rank = lane.acquisition.rank(decision_index);
        let paths_root = seed_root
            .join("native")
            .join(format!("decision-{decision_index:06}"));
        fs::create_dir_all(&paths_root).map_err(route_error)?;
        let source_snapshot = campaign.current.snapshot.clone();
        let source_route_tape = campaign.route_tape.clone();
        let restoration = campaign
            .current_restoration_contract()
            .map_err(route_error)?;
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
        let source_snapshot_sha256 = source_snapshot.content_sha256().map_err(route_error)?;
        let matching_cached_frontier = cached_frontier.as_ref().filter(|frontier| {
            frontier.state_sha256 == source_snapshot_sha256
                && frontier.route_frames == source_route_tape.frames.len()
                && frontier.route_checkpoint_sha256
                    == restoration.plan.route.route_checkpoint_sha256
                && frontier.route_tape_sha256 == restoration.plan.route.tape_sha256
        });
        let checkpoint_owner_worker_slot =
            matching_cached_frontier.map(|frontier| frontier.worker_slot);
        let usable_cached_frontier =
            matching_cached_frontier.filter(|_| pool.direct_restore_enabled);
        let directly_restored_frontier = usable_cached_frontier.is_some();
        let restore_source = if directly_restored_frontier {
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
        let terminal_projection_started = Instant::now();
        let terminal_candidates = evaluated
            .iter()
            .filter(|proposal| proposal.outcome.terminal)
            .map(|proposal| {
                campaign
                    .final_result_from_evaluated_terminal(proposal)
                    .map_err(route_error)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let terminal_projection_micros = elapsed_micros(terminal_projection_started.elapsed());
        // The policy-selected proposal is authoritative. Sibling proposals are
        // native training and candidate evidence, not an outcome-peeking beam
        // search that replaces the learner's action after observing results.
        let winner_index = 0;
        let winning_outcome = evaluated[winner_index].outcome.clone();
        let expected_transition = evaluated[winner_index].transition.clone();
        let decision = TacticQDecision {
            ranking: proposal_batch.ranking,
            selected: winning_outcome.selected.clone(),
        };
        let evaluated_native_ticks = evaluated.iter().fold(0_u64, |total, proposal| {
            total.saturating_add(u64::from(
                proposal.outcome.execution.duration.realized_ticks,
            ))
        });
        let batch_graph_admission_started = Instant::now();
        let newly_admitted_training_rows = match campaign.admit_leased_evaluated_replay(
            &evaluated,
            &evaluated_episode_groups,
            &proposal_leases,
        ) {
            Ok(rows) => rows as u64,
            Err(error) => {
                lease_ledger.resolve(lease_batch_sha256, NativeTacticLeaseOutcome::Failed)?;
                return Err(error.into());
            }
        };
        let batch_graph_admission_micros = elapsed_micros(batch_graph_admission_started.elapsed());
        let duplicate_training_transitions = evaluated
            .len()
            .saturating_sub(newly_admitted_training_rows as usize)
            as u64;
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
            promoted_tactics,
        )?;
        let next_action_catalog_micros = elapsed_micros(next_action_catalog_started.elapsed());
        record_orchestration_detail(
            &mut timing,
            OrchestrationPhase::ActionCatalogConstruction,
            next_action_catalog_micros,
        )?;
        let graph_admission_started = Instant::now();
        let step = if config.execution_plan.proposal_policy == TacticProposalPolicy::FrozenPolicy {
            campaign
                .retain_rewarded_without_policy_update(
                    decision,
                    winning_outcome.clone(),
                    &next_proposals.catalog,
                    &next_proposals.blueprints,
                    registry,
                    &encode,
                    |_| true,
                    reward_spec,
                )
                .map_err(route_error)?
        } else {
            campaign
                .retain_and_refit_rewarded(
                    decision,
                    winning_outcome.clone(),
                    &next_proposals.catalog,
                    &next_proposals.blueprints,
                    registry,
                    &encode,
                    |_| true,
                    reward_spec,
                    false,
                )
                .map_err(route_error)?
        };
        let selected_outcome_retention_micros = elapsed_micros(graph_admission_started.elapsed());
        timing.graph_admission_micros = timing
            .graph_admission_micros
            .saturating_add(selected_outcome_retention_micros);
        let frontier_retention_started = Instant::now();
        if step.step.transition != *expected_transition
            || step.reward != evaluated[winner_index].reward
        {
            return Err(route_message(
                "retained tactic proposal differs from its pre-admission evaluation",
            ));
        }
        let retained_restoration = (!winning_outcome.terminal)
            .then(|| campaign.current_restoration_contract().map_err(route_error))
            .transpose()?;
        cached_frontier = match (
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
                    retained: index == winner_index,
                })
            })
            .collect::<Result<Vec<_>, NativeTacticRouteRunError>>()?;
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
        let best_authenticated_tick_after_decision = campaign
            .best_graph_terminal_path()
            .map_err(route_error)?
            .and_then(|path| path.root_to_terminal_ticks.checked_sub(1));
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
            scheduler_decision: Some(scheduler_decision),
            branch_acquisition: branch_acquisition.take(),
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
        if let Some(session) = replay_session.as_ref() {
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
