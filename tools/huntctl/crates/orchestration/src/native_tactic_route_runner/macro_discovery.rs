use super::*;

pub(super) struct MinedTacticMacros {
    registry: TacticMacroPromotionRegistry,
    report: NativeTacticMacroDiscoveryReport,
}

pub(super) fn mine_and_store_tactic_macros(
    output_root: &Path,
    exploration_seeds: &[u64],
) -> Result<MinedTacticMacros, NativeTacticRouteRunError> {
    let mut observations = Vec::new();
    let mut observation_count = 0_u64;
    let mut high_value_observation_count = 0_u64;
    for (seed_index, seed) in exploration_seeds.iter().copied().enumerate() {
        let seed_root = output_root.join(format!("seed-{seed_index:03}-{seed}"));
        let records = read_tactic_decision_records(&seed_root)?;
        if records.is_empty() {
            continue;
        }
        let store = TacticQContentStore::open(tactic_content_store_path(&seed_root))
            .map_err(route_error)?;
        let root_tape = store.load_tape(records[0].root_tape).map_err(route_error)?;
        for record in records {
            for proposal in record.proposal_batch {
                let transition = journal_transition(
                    &store,
                    proposal.transition,
                    proposal.inline_transition.as_ref(),
                )?;
                if transition.before_state_sha256 == Digest::ZERO
                    || transition.before_state_sha256
                        != transition.before.content_sha256().map_err(route_error)?
                    || transition.execution.emitted_raw_actions.is_empty()
                    || transition.execution.emitted_raw_actions.len()
                        != transition.execution.duration.realized_ticks as usize
                    || proposal.trace.option_id != transition.value_sample.action.option_id
                {
                    return Err(route_message(
                        "tactic macro discovery source transition is detached",
                    ));
                }
                observation_count = observation_count.saturating_add(1);
                let observation = MacroDiscoveryObservation {
                    seed,
                    frontier_state_sha256: transition.before_state_sha256,
                    transition_sha256: journal_transition_sha256(
                        proposal.transition,
                        proposal.inline_transition.as_ref(),
                    )?,
                    option_id: transition.value_sample.action.option_id,
                    tape: InputTape {
                        boot: root_tape.boot.clone(),
                        tick_rate_numerator: root_tape.tick_rate_numerator,
                        tick_rate_denominator: root_tape.tick_rate_denominator,
                        frames: transition.execution.emitted_raw_actions,
                    },
                    reward: proposal.trace.reward,
                    goal_progress: record.goal_distance_before - proposal.trace.goal_distance_after,
                    terminal: proposal.trace.terminal,
                };
                if observation.tape.frames.len() <= MAX_DISCOVERED_MACRO_TICKS {
                    if observation.terminal || observation.reward > 0.0 {
                        high_value_observation_count =
                            high_value_observation_count.saturating_add(1);
                    }
                    observations.push(observation);
                    if observations.len() >= MAX_DISCOVERY_OBSERVATIONS.saturating_mul(2) {
                        retain_bounded_macro_observations(&mut observations);
                    }
                }
            }
        }
    }
    retain_bounded_macro_observations(&mut observations);
    let mut candidates = if observations.is_empty() {
        Vec::new()
    } else {
        discover_replay_macros(&observations).map_err(route_error)?
    };
    candidates.extend(mine_connected_tactic_macro_compositions(
        output_root,
        exploration_seeds,
    )?);
    let mut deduplicated = BTreeMap::<Digest, DiscoveredMacroCandidate>::new();
    for candidate in candidates {
        match deduplicated.remove(&candidate.candidate_sha256) {
            Some(existing) => {
                let mut sources = existing.sources;
                sources.extend(candidate.sources);
                deduplicated.insert(
                    candidate.candidate_sha256,
                    replay_macro_candidate(candidate.tape, sources).map_err(route_error)?,
                );
            }
            None => {
                deduplicated.insert(candidate.candidate_sha256, candidate);
            }
        }
    }
    let mut candidates = deduplicated.into_values().collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        right
            .tape
            .frames
            .len()
            .cmp(&left.tape.frames.len())
            .then_with(|| right.sources.len().cmp(&left.sources.len()))
            .then_with(|| left.candidate_sha256.cmp(&right.candidate_sha256))
    });
    candidates.truncate(MAX_DISCOVERED_MACROS);
    let mut registry = TacticMacroPromotionRegistry::default();
    for candidate in candidates {
        registry.propose(candidate).map_err(route_error)?;
    }
    let registry_path =
        output_root.join(format!("tactic-macros.{TACTIC_MACRO_REGISTRY_EXTENSION}"));
    let registry_sha256 =
        write_tactic_macro_registry(&registry_path, &registry).map_err(route_error)?;
    let restored = read_tactic_macro_registry(&registry_path).map_err(route_error)?;
    if restored.content_sha256 != registry_sha256 || restored.registry != registry {
        return Err(route_message(
            "persisted tactic macro registry failed exact round-trip verification",
        ));
    }
    let (proposed_count, promoted_count, demoted_count) =
        registry
            .records()
            .fold((0_u64, 0_u64, 0_u64), |mut counts, record| {
                match record.status {
                    MacroPromotionStatus::Proposed => counts.0 += 1,
                    MacroPromotionStatus::Promoted => counts.1 += 1,
                    MacroPromotionStatus::Demoted => counts.2 += 1,
                }
                counts
            });
    Ok(MinedTacticMacros {
        registry,
        report: NativeTacticMacroDiscoveryReport {
            observation_count,
            high_value_observation_count,
            mined_observation_count: observations.len() as u64,
            candidate_count: restored.registry.records().len() as u64,
            proposed_count,
            promoted_count,
            demoted_count,
            validation_state_count: 0,
            comparison_count: 0,
            reused_primitive_baseline_count: 0,
            validation_native_ticks: 0,
            validation_wall_micros: 0,
            validation_native_simulation_micros: 0,
            validation_preparation_micros: 0,
            validation_restore_accounting: NativeTacticRestoreAccounting::default(),
            reuse: None,
            registry_path: path_text(&registry_path),
            registry_sha256,
        },
    })
}

pub(super) fn mine_connected_tactic_macro_compositions(
    output_root: &Path,
    exploration_seeds: &[u64],
) -> Result<Vec<DiscoveredMacroCandidate>, NativeTacticRouteRunError> {
    let mut candidates = Vec::new();
    for (seed_index, seed) in exploration_seeds.iter().copied().enumerate() {
        let seed_root = output_root.join(format!("seed-{seed_index:03}-{seed}"));
        let replay = load_tactic_journal_replay(&seed_root)?;
        let store = TacticQContentStore::open(tactic_content_store_path(&seed_root))
            .map_err(route_error)?;
        let root_tape = store
            .load_tape(replay.records[0].root_tape)
            .map_err(route_error)?;
        for start in 0..replay.transitions.len() {
            let mut frames = Vec::new();
            let mut sources = Vec::new();
            for end in start..replay.transitions.len() {
                if end > start {
                    let prior = &replay.transitions[end - 1];
                    let current = &replay.transitions[end];
                    if prior.next_checkpoint_sha256 != current.source_checkpoint_sha256
                        || prior.after_state_sha256 != current.before_state_sha256
                    {
                        break;
                    }
                }
                let transition = &replay.transitions[end];
                if frames
                    .len()
                    .saturating_add(transition.execution.emitted_raw_actions.len())
                    > MAX_DISCOVERED_MACRO_TICKS
                {
                    break;
                }
                frames.extend_from_slice(&transition.execution.emitted_raw_actions);
                let record = &replay.records[end];
                sources.push(MacroSourceProvenance {
                    seed,
                    frontier_state_sha256: transition.before_state_sha256,
                    transition_sha256: journal_transition_sha256(
                        record.transition,
                        record.inline_transition.as_ref(),
                    )?,
                    option_id: transition.value_sample.action.option_id.clone(),
                });
                if sources.len() >= 2 {
                    candidates.push(
                        replay_macro_candidate(
                            InputTape {
                                boot: root_tape.boot.clone(),
                                tick_rate_numerator: root_tape.tick_rate_numerator,
                                tick_rate_denominator: root_tape.tick_rate_denominator,
                                frames: frames.clone(),
                            },
                            sources.clone(),
                        )
                        .map_err(route_error)?,
                    );
                }
            }
        }
    }
    Ok(candidates)
}

pub(super) fn retain_bounded_macro_observations(observations: &mut Vec<MacroDiscoveryObservation>) {
    if observations.len() <= MAX_DISCOVERY_OBSERVATIONS {
        return;
    }
    observations.sort_by(|left, right| {
        right
            .terminal
            .cmp(&left.terminal)
            .then_with(|| left.transition_sha256.cmp(&right.transition_sha256))
            .then_with(|| left.frontier_state_sha256.cmp(&right.frontier_state_sha256))
    });
    observations.truncate(MAX_DISCOVERY_OBSERVATIONS);
}

pub(super) struct TacticMacroValidationFrontier {
    pub(super) seed: u64,
    pub(super) state_sha256: Digest,
    pub(super) snapshot: FactSnapshot,
    pub(super) route_tape: InputTape,
    pub(super) primitive_baseline: TacticMacroMeasuredOutcome,
}

#[derive(Clone, Copy)]
pub(super) struct TacticMacroMeasuredOutcome {
    pub(super) terminal: bool,
    pub(super) progress: f32,
    pub(super) ticks: u32,
}

#[derive(Default)]
pub(super) struct TacticMacroValidationAccounting {
    native_ticks: u64,
    native_simulation_micros: u64,
    preparation_micros: u64,
    restore: NativeTacticRestoreAccounting,
}

pub(super) fn validate_and_store_tactic_macros(
    config: &NativeTacticRouteRunConfig<'_>,
    pool: &NativeTacticProposalPool,
    encoder: &GoalConditionedTacticFeatureEncoder,
    root_checkpoint_sha256: Digest,
    mut mined: MinedTacticMacros,
) -> Result<NativeTacticMacroDiscoveryReport, NativeTacticRouteRunError> {
    let started = Instant::now();
    let candidates = mined
        .registry
        .records()
        .map(|record| record.candidate.clone())
        .collect::<Vec<_>>();
    let validation_frontiers =
        if tactic_macro_promotion_has_seed_support(&config.execution_plan.seeds) {
            collect_tactic_macro_validation_frontiers(
                config.output_root,
                &config.execution_plan.seeds,
                root_checkpoint_sha256,
                encoder,
            )?
        } else {
            Vec::new()
        };
    let mut accounting = TacticMacroValidationAccounting::default();
    let mut validation_state_count = 0_u64;
    let mut comparison_count = 0_u64;
    let mut reused_primitive_baseline_count = 0_u64;
    for candidate in candidates {
        let mut used_seeds = BTreeSet::new();
        let mut used_states = BTreeSet::new();
        let mut comparison_index = 0_u64;
        for frontier in &validation_frontiers {
            if used_seeds.contains(&frontier.seed) || used_states.contains(&frontier.state_sha256) {
                continue;
            }
            let suffix_ticks = frontier
                .route_tape
                .frames
                .len()
                .saturating_sub(pool.root_source_frame) as u64;
            if !selected_tactic_fits_horizon(
                suffix_ticks,
                candidate.tape.frames.len() as u32,
                config.optimization.budgets.exploration_horizon_ticks,
            ) {
                continue;
            }
            let primitive_outcome = frontier.primitive_baseline;
            reused_primitive_baseline_count = reused_primitive_baseline_count.saturating_add(1);
            let candidate_entry = candidate.catalog_entry().map_err(route_error)?;
            let candidate_catalog =
                Arc::new(TacticAssetCatalog::new(vec![candidate_entry]).map_err(route_error)?);
            let candidate_proposals = candidate_catalog
                .option_descriptors()
                .cloned()
                .map(|descriptor| SelectedTactic {
                    schema: TACTIC_EXPLORATION_SCHEMA_V1.into(),
                    learner_snapshot_sha256: frontier.state_sha256,
                    decision_index: comparison_index,
                    descriptor,
                    reason: TacticSelectionReason::BatchDiversity,
                    exploration_draw: 0,
                })
                .collect::<Vec<_>>();
            let validation_root = config
                .output_root
                .join("tactic-macro-validation")
                .join(candidate.candidate_sha256.to_string())
                .join(format!(
                    "seed-{}-comparison-{comparison_index:02}",
                    frontier.seed
                ));
            let outcomes = evaluate_tactic_macro_validation_batch(
                pool,
                candidate_catalog,
                Arc::new(Vec::new()),
                &candidate_proposals,
                frontier,
                encoder,
                &validation_root,
                &mut accounting,
            )?;
            let [(option_id, candidate_outcome)] = outcomes.as_slice() else {
                return Err(route_message(
                    "macro validation candidate batch did not produce one outcome",
                ));
            };
            if option_id != &candidate.option_id {
                return Err(route_message(
                    "macro validation candidate outcome identity is detached",
                ));
            }
            mined
                .registry
                .observe(
                    MacroComparisonEvidence::new(
                        candidate.candidate_sha256,
                        frontier.seed,
                        frontier.state_sha256,
                        candidate_outcome.terminal,
                        candidate_outcome.progress,
                        candidate_outcome.ticks,
                        primitive_outcome.terminal,
                        primitive_outcome.progress,
                        primitive_outcome.ticks,
                    )
                    .map_err(route_error)?,
                )
                .map_err(route_error)?;
            used_seeds.insert(frontier.seed);
            used_states.insert(frontier.state_sha256);
            validation_state_count = validation_state_count.saturating_add(1);
            comparison_count = comparison_count.saturating_add(1);
            comparison_index = comparison_index.saturating_add(1);
            if comparison_index >= 2 {
                break;
            }
        }
    }
    let validated_path = config.output_root.join(format!(
        "tactic-macros-validated.{TACTIC_MACRO_REGISTRY_EXTENSION}"
    ));
    let registry_sha256 =
        write_tactic_macro_registry(&validated_path, &mined.registry).map_err(route_error)?;
    let restored = read_tactic_macro_registry(&validated_path).map_err(route_error)?;
    if restored.content_sha256 != registry_sha256 || restored.registry != mined.registry {
        return Err(route_message(
            "validated tactic macro registry failed exact round-trip verification",
        ));
    }
    let reuse = reuse_promoted_tactic_macro(
        config,
        pool,
        encoder,
        &validation_frontiers,
        &mined.registry,
        registry_sha256,
        &mut accounting,
    )?;
    let (proposed_count, promoted_count, demoted_count) =
        mined
            .registry
            .records()
            .fold((0_u64, 0_u64, 0_u64), |mut counts, record| {
                match record.status {
                    MacroPromotionStatus::Proposed => counts.0 += 1,
                    MacroPromotionStatus::Promoted => counts.1 += 1,
                    MacroPromotionStatus::Demoted => counts.2 += 1,
                }
                counts
            });
    mined.report.proposed_count = proposed_count;
    mined.report.promoted_count = promoted_count;
    mined.report.demoted_count = demoted_count;
    mined.report.validation_state_count = validation_state_count;
    mined.report.comparison_count = comparison_count;
    mined.report.reused_primitive_baseline_count = reused_primitive_baseline_count;
    mined.report.validation_native_ticks = accounting.native_ticks;
    mined.report.validation_wall_micros = elapsed_micros(started.elapsed());
    mined.report.validation_native_simulation_micros = accounting.native_simulation_micros;
    mined.report.validation_preparation_micros = accounting.preparation_micros;
    mined.report.validation_restore_accounting = accounting.restore;
    mined.report.reuse = reuse;
    mined.report.registry_path = path_text(&validated_path);
    mined.report.registry_sha256 = registry_sha256;
    Ok(mined.report)
}

pub(super) fn tactic_macro_promotion_has_seed_support(exploration_seeds: &[u64]) -> bool {
    exploration_seeds
        .iter()
        .copied()
        .collect::<BTreeSet<_>>()
        .len()
        >= MIN_PROMOTION_COMPARISONS
}

pub(super) fn reuse_promoted_tactic_macro(
    config: &NativeTacticRouteRunConfig<'_>,
    pool: &NativeTacticProposalPool,
    encoder: &GoalConditionedTacticFeatureEncoder,
    validation_frontiers: &[TacticMacroValidationFrontier],
    registry: &TacticMacroPromotionRegistry,
    registry_sha256: Digest,
    accounting: &mut TacticMacroValidationAccounting,
) -> Result<Option<NativeTacticMacroReuseReport>, NativeTacticRouteRunError> {
    let Some(promoted) = registry.promoted().next() else {
        return Ok(None);
    };
    let promotion_states = promoted
        .comparisons
        .iter()
        .map(|comparison| comparison.frontier_state_sha256)
        .collect::<BTreeSet<_>>();
    let fits = |frontier: &&TacticMacroValidationFrontier| {
        let suffix_ticks = frontier
            .route_tape
            .frames
            .len()
            .saturating_sub(pool.root_source_frame) as u64;
        selected_tactic_fits_horizon(
            suffix_ticks,
            promoted.candidate.tape.frames.len() as u32,
            config.optimization.budgets.exploration_horizon_ticks,
        )
    };
    let frontier = validation_frontiers
        .iter()
        .filter(fits)
        .find(|frontier| !promotion_states.contains(&frontier.state_sha256))
        .or_else(|| validation_frontiers.iter().find(fits))
        .ok_or_else(|| route_message("promoted tactic has no reusable authenticated frontier"))?;
    let held_out_from_promotion_states = !promotion_states.contains(&frontier.state_sha256);
    let candidate_entry = promoted.candidate.catalog_entry().map_err(route_error)?;
    let catalog = Arc::new(TacticAssetCatalog::new(vec![candidate_entry]).map_err(route_error)?);
    let selected = SelectedTactic {
        schema: TACTIC_EXPLORATION_SCHEMA_V1.into(),
        learner_snapshot_sha256: frontier.state_sha256,
        decision_index: config.execution_plan.budgets.decisions_per_lane,
        descriptor: catalog
            .option_descriptors()
            .next()
            .cloned()
            .ok_or_else(|| route_message("promoted tactic catalog is empty"))?,
        reason: TacticSelectionReason::Greedy,
        exploration_draw: 0,
    };
    let reuse_root = config
        .output_root
        .join("tactic-macro-reuse")
        .join(promoted.candidate.candidate_sha256.to_string());
    fs::create_dir_all(&reuse_root).map_err(route_error)?;
    let mut work = pool.execute_batch(
        std::slice::from_ref(&selected),
        catalog,
        Arc::new(Vec::new()),
        &frontier.snapshot,
        &frontier.route_tape,
        None,
        &reuse_root,
    )?;
    if work.len() != 1 {
        return Err(route_message(
            "promoted tactic reuse did not produce one native outcome",
        ));
    }
    let evaluated = work.remove(0);
    accounting.native_simulation_micros = accounting
        .native_simulation_micros
        .saturating_add(elapsed_micros(evaluated.native_elapsed));
    accounting.preparation_micros = accounting
        .preparation_micros
        .saturating_add(elapsed_micros(evaluated.preparation_elapsed));
    accounting.restore.merge(&evaluated.restore_accounting);
    accounting.native_ticks = accounting.native_ticks.saturating_add(u64::from(
        evaluated.outcome.execution.duration.realized_ticks,
    ));
    let emitted_tape = exact_realized_macro_tape(
        &promoted.candidate.tape,
        &evaluated.outcome.execution.emitted_raw_actions,
    )?;
    let mut expected_route = frontier.route_tape.clone();
    expected_route
        .frames
        .extend_from_slice(&emitted_tape.frames);
    if evaluated.outcome.selected.descriptor.option_id != promoted.candidate.option_id
        || evaluated.outcome.route_tape != expected_route
    {
        return Err(route_message(
            "promoted tactic reuse differs from its exact binary candidate",
        ));
    }
    let before_distance =
        encoder.encode(&frontier.snapshot).map_err(route_error)?[encoder.goal_distance_feature()];
    let after_distance = encoder
        .encode(&evaluated.outcome.next_facts)
        .map_err(route_error)?[encoder.goal_distance_feature()];
    let emitted_bytes = emitted_tape.encode().map_err(route_error)?;
    let complete_route_bytes = evaluated.outcome.route_tape.encode().map_err(route_error)?;
    let complete_route_tape_path = config.output_root.join("promoted-reuse.tape");
    if complete_route_tape_path.exists() {
        if fs::read(&complete_route_tape_path).map_err(route_error)? != complete_route_bytes {
            return Err(route_message(
                "promoted tactic reuse tape path contains different immutable content",
            ));
        }
    } else {
        write_new(&complete_route_tape_path, &complete_route_bytes)?;
    }
    Ok(Some(NativeTacticMacroReuseReport {
        candidate_sha256: promoted.candidate.candidate_sha256,
        option_id: promoted.candidate.option_id.clone(),
        promotion_registry_sha256: registry_sha256,
        seed: frontier.seed,
        source_state_sha256: frontier.state_sha256,
        held_out_from_promotion_states,
        realized_ticks: evaluated.outcome.execution.duration.realized_ticks,
        goal_progress: before_distance - after_distance,
        terminal: evaluated.outcome.terminal,
        after_state_sha256: evaluated
            .outcome
            .next_facts
            .content_sha256()
            .map_err(route_error)?,
        emitted_tape_sha256: Digest(Sha256::digest(&emitted_bytes).into()),
        complete_route_tape_sha256: Digest(Sha256::digest(&complete_route_bytes).into()),
        complete_route_tape_path: path_text(&complete_route_tape_path),
    }))
}

pub(super) fn exact_realized_macro_tape(
    candidate: &InputTape,
    emitted: &[InputFrame],
) -> Result<InputTape, NativeTacticRouteRunError> {
    if emitted.is_empty()
        || emitted.len() > candidate.frames.len()
        || candidate.frames[..emitted.len()] != *emitted
    {
        return Err(route_message(
            "promoted tactic reuse differs from its exact binary candidate prefix",
        ));
    }
    let mut realized = candidate.clone();
    realized.frames.truncate(emitted.len());
    Ok(realized)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn evaluate_tactic_macro_validation_batch(
    pool: &NativeTacticProposalPool,
    catalog: Arc<TacticAssetCatalog>,
    blueprints: Arc<Vec<TacticBlueprint>>,
    proposals: &[SelectedTactic],
    frontier: &TacticMacroValidationFrontier,
    encoder: &GoalConditionedTacticFeatureEncoder,
    output_root: &Path,
    accounting: &mut TacticMacroValidationAccounting,
) -> Result<Vec<(String, TacticMacroMeasuredOutcome)>, NativeTacticRouteRunError> {
    fs::create_dir_all(output_root).map_err(route_error)?;
    let work = pool.execute_batch(
        proposals,
        catalog,
        blueprints,
        &frontier.snapshot,
        &frontier.route_tape,
        None,
        output_root,
    )?;
    let before_distance =
        encoder.encode(&frontier.snapshot).map_err(route_error)?[encoder.goal_distance_feature()];
    work.into_iter()
        .map(|evaluated| {
            accounting.native_simulation_micros = accounting
                .native_simulation_micros
                .saturating_add(elapsed_micros(evaluated.native_elapsed));
            accounting.preparation_micros = accounting
                .preparation_micros
                .saturating_add(elapsed_micros(evaluated.preparation_elapsed));
            accounting.restore.merge(&evaluated.restore_accounting);
            let ticks = evaluated.outcome.execution.duration.realized_ticks;
            accounting.native_ticks = accounting.native_ticks.saturating_add(u64::from(ticks));
            let after_distance = encoder
                .encode(&evaluated.outcome.next_facts)
                .map_err(route_error)?[encoder.goal_distance_feature()];
            Ok((
                evaluated.outcome.selected.descriptor.option_id,
                TacticMacroMeasuredOutcome {
                    terminal: evaluated.outcome.terminal,
                    progress: before_distance - after_distance,
                    ticks,
                },
            ))
        })
        .collect()
}

pub(super) fn collect_tactic_macro_validation_frontiers(
    output_root: &Path,
    exploration_seeds: &[u64],
    root_checkpoint_sha256: Digest,
    encoder: &GoalConditionedTacticFeatureEncoder,
) -> Result<Vec<TacticMacroValidationFrontier>, NativeTacticRouteRunError> {
    let mut frontiers = BTreeMap::new();
    for (seed_index, seed) in exploration_seeds.iter().copied().enumerate() {
        let seed_root = output_root.join(format!("seed-{seed_index:03}-{seed}"));
        let replay = load_tactic_journal_replay(&seed_root)?;
        if replay.root_checkpoint_sha256 != root_checkpoint_sha256 {
            return Err(route_message(
                "macro validation frontier has a detached authenticated root",
            ));
        }
        let store = TacticQContentStore::open(tactic_content_store_path(&seed_root))
            .map_err(route_error)?;
        for (index, record) in replay.records.iter().enumerate() {
            let Some(first_proposal) = record.proposal_batch.first() else {
                continue;
            };
            let first_transition = journal_transition(
                &store,
                first_proposal.transition,
                first_proposal.inline_transition.as_ref(),
            )?;
            let mut source_route = replay
                .routes
                .get(index)
                .cloned()
                .ok_or_else(|| route_message("macro validation source route is absent"))?;
            let source_frame =
                usize::try_from(first_transition.execution.realized_tape_range.start_frame)
                    .map_err(route_error)?;
            if source_frame > source_route.frames.len() {
                return Err(route_message(
                    "macro validation source frame is beyond its retained route",
                ));
            }
            source_route.frames.truncate(source_frame);
            source_route.validate().map_err(route_error)?;
            if route_checkpoint(root_checkpoint_sha256, &source_route).map_err(route_error)?
                != first_transition.source_checkpoint_sha256
            {
                return Err(route_message(
                    "macro validation route does not reconstruct its source checkpoint",
                ));
            }
            let before_distance = encoder
                .encode(&first_transition.before)
                .map_err(route_error)?[encoder.goal_distance_feature()];
            let mut primitive_baseline = None;
            for proposal in &record.proposal_batch {
                let transition = journal_transition(
                    &store,
                    proposal.transition,
                    proposal.inline_transition.as_ref(),
                )?;
                if transition.before_state_sha256 != first_transition.before_state_sha256
                    || transition.source_checkpoint_sha256
                        != first_transition.source_checkpoint_sha256
                    || transition.execution.realized_tape_range.start_frame
                        != first_transition.execution.realized_tape_range.start_frame
                {
                    return Err(route_message(
                        "macro validation proposal batch does not share one source frontier",
                    ));
                }
                let mut endpoint_route = source_route.clone();
                endpoint_route
                    .frames
                    .extend_from_slice(&transition.execution.emitted_raw_actions);
                endpoint_route.validate().map_err(route_error)?;
                if route_checkpoint(root_checkpoint_sha256, &endpoint_route).map_err(route_error)?
                    != transition.next_checkpoint_sha256
                {
                    return Err(route_message(
                        "macro validation proposal endpoint route is detached",
                    ));
                }
                let after_distance = encoder.encode(&transition.after).map_err(route_error)?
                    [encoder.goal_distance_feature()];
                let outcome = TacticMacroMeasuredOutcome {
                    terminal: transition.value_sample.terminal
                        || transition.after.terminal.reached == Some(true),
                    progress: before_distance - after_distance,
                    ticks: transition.execution.duration.realized_ticks,
                };
                primitive_baseline = Some(match primitive_baseline {
                    Some(best) if !tactic_macro_outcome_is_better(outcome, best) => best,
                    _ => outcome,
                });
            }
            if first_transition.before.terminal.reached != Some(true) {
                insert_tactic_macro_validation_frontier(
                    &mut frontiers,
                    TacticMacroValidationFrontier {
                        seed,
                        state_sha256: first_transition.before_state_sha256,
                        snapshot: first_transition.before,
                        route_tape: source_route,
                        primitive_baseline: primitive_baseline.ok_or_else(|| {
                            route_message("macro validation frontier has no primitive comparison")
                        })?,
                    },
                )?;
            }
        }
    }
    Ok(frontiers.into_values().collect())
}

pub(super) fn insert_tactic_macro_validation_frontier(
    frontiers: &mut BTreeMap<(u64, Digest, Digest), TacticMacroValidationFrontier>,
    frontier: TacticMacroValidationFrontier,
) -> Result<(), NativeTacticRouteRunError> {
    // Fact snapshots are deliberately compact learner observations, not full
    // emulator-state identities. Different input lineages can legitimately
    // converge on the same visible snapshot, while their authenticated tapes
    // remain distinct replay frontiers. Keep both; only conflicting evidence
    // for the same exact route identity is corruption.
    let route_sha256 =
        Digest(Sha256::digest(frontier.route_tape.encode().map_err(route_error)?).into());
    let identity = (frontier.seed, frontier.state_sha256, route_sha256);
    match frontiers.get_mut(&identity) {
        Some(existing)
            if existing.snapshot != frontier.snapshot
                || existing.route_tape != frontier.route_tape =>
        {
            Err(route_message(
                "macro validation frontier identity has conflicting replay evidence",
            ))
        }
        Some(existing) => {
            // Revisited frontiers can evaluate different primitive batches.
            // Merge their strongest authenticated terminal/tick baseline
            // instead of treating ordinary exploration as identity corruption.
            let candidate = frontier.primitive_baseline;
            let incumbent = existing.primitive_baseline;
            if tactic_macro_outcome_is_better(candidate, incumbent) {
                existing.primitive_baseline = candidate;
            }
            Ok(())
        }
        None => {
            frontiers.insert(identity, frontier);
            Ok(())
        }
    }
}

pub(super) fn tactic_macro_outcome_is_better(
    candidate: TacticMacroMeasuredOutcome,
    incumbent: TacticMacroMeasuredOutcome,
) -> bool {
    candidate
        .terminal
        .cmp(&incumbent.terminal)
        .then_with(|| incumbent.ticks.cmp(&candidate.ticks))
        .is_gt()
}
