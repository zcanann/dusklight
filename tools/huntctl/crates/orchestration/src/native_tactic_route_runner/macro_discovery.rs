use super::*;

pub(super) struct MinedTacticMacros {
    registry: TacticMacroPromotionRegistry,
    report: NativeTacticMacroDiscoveryReport,
}

pub(super) struct ValidatedTacticMacros {
    registry: TacticMacroPromotionRegistry,
    report: NativeTacticMacroDiscoveryReport,
}

pub(super) struct ActiveTacticMacroRefresh {
    pub(super) promoted_tactics: Vec<ImportedPromotedTactic>,
    pub(super) report: NativeTacticMacroDiscoveryReport,
}

#[derive(Default)]
pub(super) struct ActiveTacticMacroLifecycle {
    pub(super) validation_reports: Vec<NativeTacticMacroDiscoveryReport>,
    pub(super) promoted_option_ids: BTreeSet<String>,
    pub(super) selected_decisions: u64,
}

#[derive(Clone, Copy)]
pub(super) struct TacticMacroSourceLane {
    pub(super) seed_index: usize,
    pub(super) seed: u64,
}

pub(super) fn should_refresh_active_tactic_macros(
    proposal_policy: TacticProposalPolicy,
    generation_position: usize,
    generation_count: usize,
) -> bool {
    proposal_policy == TacticProposalPolicy::Learned
        && generation_position.saturating_add(1) < generation_count
}

pub(super) fn count_active_tactic_selections(
    output_root: &Path,
    source_lanes: &[TacticMacroSourceLane],
    promoted_option_ids: &BTreeSet<String>,
) -> Result<u64, NativeTacticRouteRunError> {
    if promoted_option_ids.is_empty() {
        return Ok(0);
    }
    source_lanes.iter().try_fold(0_u64, |total, lane| {
        let seed_root = output_root.join(format!("seed-{:03}-{}", lane.seed_index, lane.seed));
        let selected = read_tactic_decision_journal(&seed_root)?
            .into_iter()
            .filter(|decision| promoted_option_ids.contains(&decision.selected_option_id))
            .count();
        total
            .checked_add(u64::try_from(selected).map_err(route_error)?)
            .ok_or_else(|| route_message("active tactic selection count overflowed"))
    })
}

pub(super) fn finalize_tactic_macro_discovery(
    config: &NativeTacticRouteRunConfig<'_>,
    pool: &NativeTacticProposalPool,
    encoder: &GoalConditionedTacticFeatureEncoder,
    execution_plan_sha256: Digest,
    root_checkpoint_sha256: Digest,
    active: &ActiveTacticMacroLifecycle,
) -> Result<NativeTacticMacroDiscoveryReport, NativeTacticRouteRunError> {
    let durable_path = config.output_root.join(NATIVE_TACTIC_MACRO_DISCOVERY_FILE);
    if durable_path.is_file() {
        return read_macro_discovery_report(
            config.output_root,
            execution_plan_sha256,
            config.optimization.terminal_predicate.definition_sha256,
            encoder.schema_sha256,
            root_checkpoint_sha256,
        );
    }
    let source_lanes = config
        .execution_plan
        .seeds
        .iter()
        .copied()
        .enumerate()
        .map(|(seed_index, seed)| TacticMacroSourceLane { seed_index, seed })
        .collect::<Vec<_>>();
    let mined = mine_and_store_tactic_macros(
        config.output_root,
        &source_lanes,
        encoder,
        MAX_DISCOVERED_MACROS,
        config
            .output_root
            .join(format!("tactic-macros.{TACTIC_MACRO_REGISTRY_EXTENSION}")),
    )?;
    let validated = validate_and_store_tactic_macros(
        config,
        pool,
        encoder,
        root_checkpoint_sha256,
        &source_lanes,
        config.output_root.join("tactic-macro-validation"),
        config.output_root.join(format!(
            "tactic-macros-validated.{TACTIC_MACRO_REGISTRY_EXTENSION}"
        )),
        true,
        mined,
    )?;
    let mut report = validated.report;
    absorb_active_tactic_macro_validation(&mut report, &active.validation_reports);
    report.active_refresh_count =
        u64::try_from(active.validation_reports.len()).map_err(route_error)?;
    report.active_promoted_option_ids = active.promoted_option_ids.iter().cloned().collect();
    report.active_selected_decisions = active.selected_decisions;
    write_macro_discovery_report(
        config.output_root,
        execution_plan_sha256,
        config.optimization.terminal_predicate.definition_sha256,
        encoder.schema_sha256,
        root_checkpoint_sha256,
        report,
    )
}

pub(super) fn refresh_active_tactic_macros(
    config: &NativeTacticRouteRunConfig<'_>,
    pool: &NativeTacticProposalPool,
    encoder: &GoalConditionedTacticFeatureEncoder,
    root_checkpoint_sha256: Digest,
    source_lanes: &[TacticMacroSourceLane],
    generation_index: usize,
) -> Result<Option<ActiveTacticMacroRefresh>, NativeTacticRouteRunError> {
    // Active promotion is deliberately narrow: validate the strongest new
    // candidate between generations, then let the next generation provide the
    // real reuse evidence. Finalization still audits the complete bounded set.
    let active_root = config
        .output_root
        .join("tactic-macro-active")
        .join(format!("generation-{generation_index:03}"));
    let mined = mine_and_store_tactic_macros(
        config.output_root,
        source_lanes,
        encoder,
        1,
        active_root.join(format!("mined.{TACTIC_MACRO_REGISTRY_EXTENSION}")),
    )?;
    if mined.registry.records().len() == 0 {
        return Ok(None);
    }
    let validated = validate_and_store_tactic_macros(
        config,
        pool,
        encoder,
        root_checkpoint_sha256,
        source_lanes,
        active_root.join("validation"),
        active_root.join(format!("validated.{TACTIC_MACRO_REGISTRY_EXTENSION}")),
        false,
        mined,
    )?;
    let promoted_tactics = promoted_tactic_entries(&validated.registry)?;
    Ok(Some(ActiveTacticMacroRefresh {
        promoted_tactics,
        report: validated.report,
    }))
}

pub(super) fn discover_active_tactic_candidates(
    output_root: &Path,
    source_lane: TacticMacroSourceLane,
    encoder: &GoalConditionedTacticFeatureEncoder,
    decision_index: u64,
) -> Result<Vec<ImportedPromotedTactic>, NativeTacticRouteRunError> {
    let candidate_root = output_root.join("tactic-macro-candidates").join(format!(
        "seed-{:03}-{}-decision-{decision_index:06}",
        source_lane.seed_index, source_lane.seed
    ));
    fs::create_dir_all(&candidate_root).map_err(route_error)?;
    let registry_path =
        candidate_root.join(format!("candidates.{TACTIC_MACRO_REGISTRY_EXTENSION}"));
    if registry_path.is_file() {
        let restored = read_tactic_macro_registry(&registry_path).map_err(route_error)?;
        return candidate_tactic_entries(&restored.registry);
    }
    let mined =
        mine_and_store_tactic_macros(output_root, &[source_lane], encoder, 1, registry_path)?;
    candidate_tactic_entries(&mined.registry)
}

pub(super) fn load_active_tactic_candidates(
    output_root: &Path,
    source_lane: TacticMacroSourceLane,
) -> Result<Vec<ImportedPromotedTactic>, NativeTacticRouteRunError> {
    let candidate_root = output_root.join("tactic-macro-candidates");
    if !candidate_root.is_dir() {
        return Ok(Vec::new());
    }
    let prefix = format!(
        "seed-{:03}-{}-decision-",
        source_lane.seed_index, source_lane.seed
    );
    let entries = fs::read_dir(candidate_root)
        .map_err(route_error)?
        .map(|entry| entry.map_err(route_error))
        .collect::<Result<Vec<_>, _>>()?;
    let mut registry_paths = entries
        .into_iter()
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(&prefix))
        .map(|entry| {
            entry
                .path()
                .join(format!("candidates.{TACTIC_MACRO_REGISTRY_EXTENSION}"))
        })
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    registry_paths.sort();
    let mut candidates = Vec::new();
    for path in registry_paths {
        let restored = read_tactic_macro_registry(&path).map_err(route_error)?;
        merge_promoted_tactic_entries(
            &mut candidates,
            candidate_tactic_entries(&restored.registry)?,
        )?;
    }
    Ok(candidates)
}

pub(super) fn absorb_active_tactic_macro_validation(
    final_report: &mut NativeTacticMacroDiscoveryReport,
    active_reports: &[NativeTacticMacroDiscoveryReport],
) {
    for active in active_reports {
        final_report.validation_state_count = final_report
            .validation_state_count
            .saturating_add(active.validation_state_count);
        final_report.comparison_count = final_report
            .comparison_count
            .saturating_add(active.comparison_count);
        final_report.executed_component_baseline_count = final_report
            .executed_component_baseline_count
            .saturating_add(active.executed_component_baseline_count);
        final_report.validation_native_ticks = final_report
            .validation_native_ticks
            .saturating_add(active.validation_native_ticks);
        final_report.validation_wall_micros = final_report
            .validation_wall_micros
            .saturating_add(active.validation_wall_micros);
        final_report.validation_native_simulation_micros = final_report
            .validation_native_simulation_micros
            .saturating_add(active.validation_native_simulation_micros);
        final_report.validation_ipc_and_result_transport_micros = final_report
            .validation_ipc_and_result_transport_micros
            .saturating_add(active.validation_ipc_and_result_transport_micros);
        final_report.validation_native_observation_capture_micros = final_report
            .validation_native_observation_capture_micros
            .saturating_add(active.validation_native_observation_capture_micros);
        final_report.validation_native_corpus_encoding_micros = final_report
            .validation_native_corpus_encoding_micros
            .saturating_add(active.validation_native_corpus_encoding_micros);
        final_report.validation_rust_state_extraction_micros = final_report
            .validation_rust_state_extraction_micros
            .saturating_add(active.validation_rust_state_extraction_micros);
        final_report.validation_preparation_micros = final_report
            .validation_preparation_micros
            .saturating_add(active.validation_preparation_micros);
        final_report
            .validation_restore_accounting
            .merge(&active.validation_restore_accounting);
    }
}

pub(super) fn mine_and_store_tactic_macros(
    output_root: &Path,
    source_lanes: &[TacticMacroSourceLane],
    encoder: &GoalConditionedTacticFeatureEncoder,
    maximum_candidates: usize,
    registry_path: PathBuf,
) -> Result<MinedTacticMacros, NativeTacticRouteRunError> {
    if maximum_candidates == 0 || maximum_candidates > MAX_DISCOVERED_MACROS {
        return Err(route_message("tactic macro candidate capacity is invalid"));
    }
    let mut observations = Vec::new();
    let mut observation_count = 0_u64;
    let mut high_value_observation_count = 0_u64;
    for source_lane in source_lanes {
        let seed_index = source_lane.seed_index;
        let seed = source_lane.seed;
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
                let component = tactic_macro_component_for_transition(
                    seed,
                    record.decision_index,
                    &transition,
                    encoder,
                    proposal.component.as_ref(),
                )?;
                let observation = MacroDiscoveryObservation {
                    seed,
                    frontier_state_sha256: transition.before_state_sha256,
                    transition_sha256: journal_transition_sha256(
                        proposal.transition,
                        proposal.inline_transition.as_ref(),
                    )?,
                    component,
                    entry: macro_entry_observation(&transition.before, encoder)?,
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
        source_lanes,
        encoder,
    )?);
    let mut deduplicated = BTreeMap::<Digest, DiscoveredMacroCandidate>::new();
    for candidate in candidates {
        match deduplicated.remove(&candidate.candidate_sha256) {
            Some(existing) => {
                if existing.tape != candidate.tape || existing.components != candidate.components {
                    return Err(route_message(
                        "tactic macro candidate identity collides across different content",
                    ));
                }
                let mut sources = existing.sources;
                sources.extend(candidate.sources);
                deduplicated.insert(
                    candidate.candidate_sha256,
                    replay_macro_candidate(candidate.tape, candidate.components, sources)
                        .map_err(route_error)?,
                );
            }
            None => {
                deduplicated.insert(candidate.candidate_sha256, candidate);
            }
        }
    }
    let mut candidates = deduplicated.into_values().collect::<Vec<_>>();
    candidates.sort_by(compare_tactic_macro_candidate_priority);
    candidates.truncate(maximum_candidates);
    let mut registry = TacticMacroPromotionRegistry::default();
    for candidate in candidates {
        registry.propose(candidate).map_err(route_error)?;
    }
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
            active_refresh_count: 0,
            active_promoted_option_ids: Vec::new(),
            active_selected_decisions: 0,
            observation_count,
            high_value_observation_count,
            mined_observation_count: observations.len() as u64,
            candidate_count: restored.registry.records().len() as u64,
            entry_condition_count: restored.registry.records().len() as u64,
            held_out_compatible_candidate_count: 0,
            source_state_exclusion_count: 0,
            entry_incompatible_frontier_count: 0,
            proposed_count,
            promoted_count,
            demoted_count,
            validation_state_count: 0,
            comparison_count: 0,
            reused_primitive_baseline_count: 0,
            executed_component_baseline_count: 0,
            validation_native_ticks: 0,
            validation_wall_micros: 0,
            validation_native_simulation_micros: 0,
            validation_ipc_and_result_transport_micros: 0,
            validation_native_observation_capture_micros: 0,
            validation_native_corpus_encoding_micros: 0,
            validation_rust_state_extraction_micros: 0,
            validation_preparation_micros: 0,
            validation_restore_accounting: NativeTacticRestoreAccounting::default(),
            reuse: None,
            registry_path: path_text(&registry_path),
            registry_sha256,
        },
    })
}

pub(super) fn compare_tactic_macro_candidate_priority(
    left: &DiscoveredMacroCandidate,
    right: &DiscoveredMacroCandidate,
) -> std::cmp::Ordering {
    // Active refresh validates only the strongest bounded candidate. Prefer
    // actual compositions: collapsing several policy decisions is an
    // attainable sample-efficiency gain, while a full-length copy of one
    // primitive normally cannot outperform its own realization.
    right
        .components
        .len()
        .cmp(&left.components.len())
        .then_with(|| right.tape.frames.len().cmp(&left.tape.frames.len()))
        .then_with(|| right.sources.len().cmp(&left.sources.len()))
        .then_with(|| left.candidate_sha256.cmp(&right.candidate_sha256))
}

fn macro_entry_observation(
    snapshot: &FactSnapshot,
    encoder: &GoalConditionedTacticFeatureEncoder,
) -> Result<MacroEntryObservation, NativeTacticRouteRunError> {
    let goal_distance =
        encoder.encode(snapshot).map_err(route_error)?[encoder.goal_distance_feature()];
    if !goal_distance.is_finite() || goal_distance < 0.0 {
        return Err(route_message(
            "macro discovery source has invalid goal distance",
        ));
    }
    Ok(MacroEntryObservation {
        stage: snapshot.world.stage.clone(),
        room: snapshot.world.room,
        player_procedure: snapshot.player.procedure,
        player_contacts: snapshot.player.contacts,
        goal_distance_f32_bits: goal_distance.to_bits(),
    })
}

fn tactic_macro_component_for_transition(
    seed: u64,
    decision_index: u64,
    transition: &OptionTransitionSample,
    encoder: &GoalConditionedTacticFeatureEncoder,
    retained: Option<&TacticMacroComponent>,
) -> Result<TacticMacroComponent, NativeTacticRouteRunError> {
    if let Some(retained) = retained {
        let entry = retained.catalog_entry().map_err(route_error)?;
        if retained.action != transition.value_sample.action
            || entry.description().option != transition.value_sample.action
        {
            return Err(route_message(
                "retained tactic macro component is detached from its transition",
            ));
        }
        return Ok(retained.clone());
    }
    let proposals = parameterized_catalog_for_state(
        seed,
        decision_index,
        &transition.before,
        encoder,
        transition.execution.duration.maximum_ticks,
        None,
        parameterized_tactic_family_schema_sha256(),
    )?;
    let entry = proposals
        .catalog
        .entry(&transition.value_sample.action.option_id)
        .filter(|entry| entry.description().option == transition.value_sample.action)
        .ok_or_else(|| {
            route_message(
                "tactic macro component executable source cannot be reconstructed exactly",
            )
        })?;
    TacticMacroComponent::from_catalog_entry(entry).map_err(route_error)
}

pub(super) fn mine_connected_tactic_macro_compositions(
    output_root: &Path,
    source_lanes: &[TacticMacroSourceLane],
    encoder: &GoalConditionedTacticFeatureEncoder,
) -> Result<Vec<DiscoveredMacroCandidate>, NativeTacticRouteRunError> {
    let mut occurrences = ConnectedMacroOccurrences::new();
    for source_lane in source_lanes {
        let seed_index = source_lane.seed_index;
        let seed = source_lane.seed;
        let seed_root = output_root.join(format!("seed-{seed_index:03}-{seed}"));
        let replay = load_tactic_journal_replay(&seed_root)?;
        let store = TacticQContentStore::open(tactic_content_store_path(&seed_root))
            .map_err(route_error)?;
        let root_tape = store
            .load_tape(replay.records[0].root_tape)
            .map_err(route_error)?;
        for start in 0..replay.transitions.len() {
            let mut frames = Vec::new();
            let mut components = Vec::new();
            let mut transition_sha256s = Vec::new();
            let source_transition = &replay.transitions[start];
            let source_entry = macro_entry_observation(&source_transition.before, encoder)?;
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
                let retained = record
                    .proposal_batch
                    .iter()
                    .find(|proposal| proposal.trace.retained)
                    .and_then(|proposal| proposal.component.as_ref());
                components.push(tactic_macro_component_for_transition(
                    seed,
                    record.decision_index,
                    transition,
                    encoder,
                    retained,
                )?);
                transition_sha256s.push(journal_transition_sha256(
                    record.transition,
                    record.inline_transition.as_ref(),
                )?);
                if end > start {
                    let tape = InputTape {
                        boot: root_tape.boot.clone(),
                        tick_rate_numerator: root_tape.tick_rate_numerator,
                        tick_rate_denominator: root_tape.tick_rate_denominator,
                        frames: frames.clone(),
                    };
                    let source = MacroSourceProvenance {
                        seed,
                        frontier_state_sha256: source_transition.before_state_sha256,
                        transition_sha256s: transition_sha256s.clone(),
                        entry: source_entry.clone(),
                    };
                    let key = connected_macro_occurrence_key(&tape, &components)?;
                    occurrences
                        .entry(key)
                        .or_insert_with(|| (tape, components.clone(), BTreeMap::new()))
                        .2
                        .insert(source.transition_sha256s.clone(), source);
                }
            }
        }
    }
    connected_macro_candidates(occurrences)
}

pub(super) type ConnectedMacroOccurrences = BTreeMap<
    Vec<u8>,
    (
        InputTape,
        Vec<TacticMacroComponent>,
        BTreeMap<Vec<Digest>, MacroSourceProvenance>,
    ),
>;

fn connected_macro_occurrence_key(
    tape: &InputTape,
    components: &[TacticMacroComponent],
) -> Result<Vec<u8>, NativeTacticRouteRunError> {
    let encoded_tape = tape.encode().map_err(route_error)?;
    let mut key =
        Vec::with_capacity(8 + encoded_tape.len() + 8 + components.len().saturating_mul(32));
    key.extend_from_slice(&(encoded_tape.len() as u64).to_le_bytes());
    key.extend_from_slice(&encoded_tape);
    key.extend_from_slice(&(components.len() as u64).to_le_bytes());
    for component in components {
        key.extend_from_slice(&component.content_sha256().map_err(route_error)?.0);
    }
    Ok(key)
}

pub(super) fn connected_macro_candidates(
    occurrences: ConnectedMacroOccurrences,
) -> Result<Vec<DiscoveredMacroCandidate>, NativeTacticRouteRunError> {
    occurrences
        .into_values()
        .filter(|(_, _, sources)| {
            sources
                .values()
                .map(|source| source.frontier_state_sha256)
                .collect::<BTreeSet<_>>()
                .len()
                >= MIN_DISCOVERY_OCCURRENCES
        })
        .map(|(tape, components, sources)| {
            replay_macro_candidate(tape, components, sources.into_values().collect())
                .map_err(route_error)
        })
        .collect()
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

#[derive(Clone)]
pub(super) struct TacticMacroValidationFrontier {
    pub(super) seed: u64,
    pub(super) state_sha256: Digest,
    pub(super) snapshot: FactSnapshot,
    pub(super) route_tape: InputTape,
}

#[derive(Clone)]
pub(super) struct TacticMacroMeasuredOutcome {
    pub(super) terminal: bool,
    pub(super) progress: f32,
    pub(super) ticks: u32,
    pub(super) emitted_raw_actions: Vec<InputFrame>,
}

#[derive(Default)]
pub(super) struct TacticMacroValidationAccounting {
    native_ticks: u64,
    native_simulation_micros: u64,
    ipc_and_result_transport_micros: u64,
    native_observation_capture_micros: u64,
    native_corpus_encoding_micros: u64,
    rust_state_extraction_micros: u64,
    preparation_micros: u64,
    restore: NativeTacticRestoreAccounting,
}

impl TacticMacroValidationAccounting {
    fn merge(&mut self, other: &Self) {
        self.native_ticks = self.native_ticks.saturating_add(other.native_ticks);
        self.native_simulation_micros = self
            .native_simulation_micros
            .saturating_add(other.native_simulation_micros);
        self.ipc_and_result_transport_micros = self
            .ipc_and_result_transport_micros
            .saturating_add(other.ipc_and_result_transport_micros);
        self.native_observation_capture_micros = self
            .native_observation_capture_micros
            .saturating_add(other.native_observation_capture_micros);
        self.native_corpus_encoding_micros = self
            .native_corpus_encoding_micros
            .saturating_add(other.native_corpus_encoding_micros);
        self.rust_state_extraction_micros = self
            .rust_state_extraction_micros
            .saturating_add(other.rust_state_extraction_micros);
        self.preparation_micros = self
            .preparation_micros
            .saturating_add(other.preparation_micros);
        self.restore.merge(&other.restore);
    }
}
struct TacticMacroValidationJob {
    candidate: DiscoveredMacroCandidate,
    frontier: TacticMacroValidationFrontier,
    comparison_index: u64,
    output_root: PathBuf,
}

struct TacticMacroValidationResult {
    candidate_sha256: Digest,
    frontier_seed: u64,
    frontier_state_sha256: Digest,
    candidate_outcome: TacticMacroMeasuredOutcome,
    primitive_outcome: TacticMacroMeasuredOutcome,
    accounting: TacticMacroValidationAccounting,
}

pub(super) fn validate_and_store_tactic_macros(
    config: &NativeTacticRouteRunConfig<'_>,
    pool: &NativeTacticProposalPool,
    encoder: &GoalConditionedTacticFeatureEncoder,
    root_checkpoint_sha256: Digest,
    source_lanes: &[TacticMacroSourceLane],
    validation_root: PathBuf,
    validated_path: PathBuf,
    prove_immediate_reuse: bool,
    mut mined: MinedTacticMacros,
) -> Result<ValidatedTacticMacros, NativeTacticRouteRunError> {
    let started = Instant::now();
    let candidates = mined
        .registry
        .records()
        .map(|record| record.candidate.clone())
        .collect::<Vec<_>>();
    let validation_frontiers = collect_tactic_macro_validation_frontiers(
        config.output_root,
        source_lanes,
        root_checkpoint_sha256,
    )?;
    let mut jobs = Vec::new();
    let mut held_out_compatible_candidate_count = 0_u64;
    let mut source_state_exclusion_count = 0_u64;
    let mut entry_incompatible_frontier_count = 0_u64;
    for candidate in candidates {
        let component_maximum_ticks = tactic_macro_component_maximum_ticks(&candidate)?;
        let source_states = candidate
            .sources
            .iter()
            .map(|source| source.frontier_state_sha256)
            .collect::<BTreeSet<_>>();
        let entry_condition = candidate.entry_condition().map_err(route_error)?;
        let mut compatible_frontiers = Vec::new();
        for frontier in &validation_frontiers {
            if source_states.contains(&frontier.state_sha256) {
                source_state_exclusion_count = source_state_exclusion_count.saturating_add(1);
                continue;
            }
            if let Some(distance) =
                tactic_macro_entry_distance(&entry_condition, frontier, encoder)?
            {
                compatible_frontiers.push((distance, frontier));
            } else {
                entry_incompatible_frontier_count =
                    entry_incompatible_frontier_count.saturating_add(1);
            }
        }
        if !compatible_frontiers.is_empty() {
            held_out_compatible_candidate_count =
                held_out_compatible_candidate_count.saturating_add(1);
        }
        compatible_frontiers.sort_by(|left, right| {
            left.0
                .total_cmp(&right.0)
                .then_with(|| left.1.seed.cmp(&right.1.seed))
                .then_with(|| left.1.state_sha256.cmp(&right.1.state_sha256))
        });
        let mut used_states = BTreeSet::new();
        let mut comparison_index = 0_u64;
        for (_, frontier) in compatible_frontiers {
            if used_states.contains(&frontier.state_sha256) {
                continue;
            }
            let suffix_ticks = frontier
                .route_tape
                .frames
                .len()
                .saturating_sub(pool.root_source_frame) as u64;
            if !selected_tactic_fits_horizon(
                suffix_ticks,
                component_maximum_ticks,
                config.optimization.budgets.exploration_horizon_ticks,
            ) {
                continue;
            }
            let job_output_root = validation_root
                .join(candidate.candidate_sha256.to_string())
                .join(format!(
                    "seed-{}-comparison-{comparison_index:02}",
                    frontier.seed
                ));
            jobs.push(TacticMacroValidationJob {
                candidate: candidate.clone(),
                frontier: frontier.clone(),
                comparison_index,
                output_root: job_output_root,
            });
            used_states.insert(frontier.state_sha256);
            comparison_index = comparison_index.saturating_add(1);
            if comparison_index >= 2 {
                break;
            }
        }
    }
    let results = std::thread::scope(|scope| {
        let handles = jobs
            .into_iter()
            .map(|job| {
                let pool = pool.clone();
                scope.spawn(move || run_tactic_macro_validation_job(&pool, encoder, job))
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .map_err(|_| route_message("tactic macro validation worker panicked"))?
            })
            .collect::<Result<Vec<_>, NativeTacticRouteRunError>>()
    })?;
    let mut accounting = TacticMacroValidationAccounting::default();
    for result in &results {
        accounting.merge(&result.accounting);
        mined
            .registry
            .observe(
                MacroComparisonEvidence::new(
                    result.candidate_sha256,
                    result.frontier_seed,
                    result.frontier_state_sha256,
                    result.candidate_outcome.terminal,
                    result.candidate_outcome.progress,
                    result.candidate_outcome.ticks,
                    result.primitive_outcome.terminal,
                    result.primitive_outcome.progress,
                    result.primitive_outcome.ticks,
                )
                .map_err(route_error)?,
            )
            .map_err(route_error)?;
    }
    let validation_state_count = results.len() as u64;
    let comparison_count = results.len() as u64;
    let executed_component_baseline_count = results.len() as u64;
    let registry_sha256 =
        write_tactic_macro_registry(&validated_path, &mined.registry).map_err(route_error)?;
    let restored = read_tactic_macro_registry(&validated_path).map_err(route_error)?;
    if restored.content_sha256 != registry_sha256 || restored.registry != mined.registry {
        return Err(route_message(
            "validated tactic macro registry failed exact round-trip verification",
        ));
    }
    let reuse = if prove_immediate_reuse {
        reuse_promoted_tactic_macro(
            config,
            pool,
            encoder,
            &validation_frontiers,
            &mined.registry,
            registry_sha256,
            &mut accounting,
        )?
    } else {
        None
    };
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
    mined.report.held_out_compatible_candidate_count = held_out_compatible_candidate_count;
    mined.report.source_state_exclusion_count = source_state_exclusion_count;
    mined.report.entry_incompatible_frontier_count = entry_incompatible_frontier_count;
    mined.report.validation_state_count = validation_state_count;
    mined.report.comparison_count = comparison_count;
    mined.report.reused_primitive_baseline_count = 0;
    mined.report.executed_component_baseline_count = executed_component_baseline_count;
    mined.report.validation_native_ticks = accounting.native_ticks;
    mined.report.validation_wall_micros = elapsed_micros(started.elapsed());
    mined.report.validation_native_simulation_micros = accounting.native_simulation_micros;
    mined.report.validation_ipc_and_result_transport_micros =
        accounting.ipc_and_result_transport_micros;
    mined.report.validation_native_observation_capture_micros =
        accounting.native_observation_capture_micros;
    mined.report.validation_native_corpus_encoding_micros =
        accounting.native_corpus_encoding_micros;
    mined.report.validation_rust_state_extraction_micros = accounting.rust_state_extraction_micros;
    mined.report.validation_preparation_micros = accounting.preparation_micros;
    mined.report.validation_restore_accounting = accounting.restore;
    mined.report.reuse = reuse;
    mined.report.registry_path = path_text(&validated_path);
    mined.report.registry_sha256 = registry_sha256;
    Ok(ValidatedTacticMacros {
        registry: mined.registry,
        report: mined.report,
    })
}

pub(super) fn tactic_macro_entry_distance(
    condition: &dusklight_learning::tactic_macro_promotion::TacticMacroEntryCondition,
    frontier: &TacticMacroValidationFrontier,
    encoder: &GoalConditionedTacticFeatureEncoder,
) -> Result<Option<f32>, NativeTacticRouteRunError> {
    let snapshot = &frontier.snapshot;
    let distance = encoder.encode(snapshot).map_err(route_error)?[encoder.goal_distance_feature()];
    Ok(condition.distance_to_support(
        &snapshot.world.stage,
        snapshot.world.room,
        snapshot.player.procedure,
        snapshot.player.contacts,
        distance,
        TACTIC_MACRO_ENTRY_GOAL_DISTANCE_PADDING,
    ))
}

fn run_tactic_macro_validation_job(
    pool: &NativeTacticProposalPool,
    encoder: &GoalConditionedTacticFeatureEncoder,
    job: TacticMacroValidationJob,
) -> Result<TacticMacroValidationResult, NativeTacticRouteRunError> {
    let candidate_entry = job.candidate.catalog_entry().map_err(route_error)?;
    let candidate_catalog =
        Arc::new(TacticAssetCatalog::new(vec![candidate_entry]).map_err(route_error)?);
    let candidate_proposals = candidate_catalog
        .option_descriptors()
        .cloned()
        .map(|descriptor| SelectedTactic {
            schema: TACTIC_EXPLORATION_SCHEMA_V1.into(),
            learner_snapshot_sha256: job.frontier.state_sha256,
            decision_index: job.comparison_index,
            descriptor,
            reason: TacticSelectionReason::BatchDiversity,
            exploration_draw: 0,
        })
        .collect::<Vec<_>>();
    let mut accounting = TacticMacroValidationAccounting::default();
    let outcomes = evaluate_tactic_macro_validation_batch(
        pool,
        candidate_catalog,
        Arc::new(Vec::new()),
        &candidate_proposals,
        &job.frontier,
        encoder,
        &job.output_root,
        &mut accounting,
    )?;
    let [(option_id, candidate_outcome)] = outcomes.as_slice() else {
        return Err(route_message(
            "macro validation candidate batch did not produce one outcome",
        ));
    };
    if option_id != &job.candidate.option_id {
        return Err(route_message(
            "macro validation candidate outcome identity is detached",
        ));
    }
    let primitive_outcome = evaluate_tactic_macro_component_sequence(
        pool,
        &job.candidate,
        &job.frontier,
        encoder,
        &job.output_root.join("primitive-components"),
        &mut accounting,
    )?;
    exact_realized_macro_tape(&job.candidate.tape, &candidate_outcome.emitted_raw_actions)?;
    if candidate_outcome.emitted_raw_actions != primitive_outcome.emitted_raw_actions {
        return Err(route_message(
            "macro validation candidate and primitive sequence emitted different inputs",
        ));
    }
    Ok(TacticMacroValidationResult {
        candidate_sha256: job.candidate.candidate_sha256,
        frontier_seed: job.frontier.seed,
        frontier_state_sha256: job.frontier.state_sha256,
        candidate_outcome: candidate_outcome.clone(),
        primitive_outcome,
        accounting,
    })
}

fn tactic_macro_component_maximum_ticks(
    candidate: &DiscoveredMacroCandidate,
) -> Result<u32, NativeTacticRouteRunError> {
    candidate
        .components
        .iter()
        .try_fold(0_u32, |total, component| {
            let entry = component.catalog_entry().map_err(route_error)?;
            total
                .checked_add(entry.description().duration.maximum_ticks)
                .ok_or_else(|| route_message("tactic macro component duration overflows"))
        })
}

fn evaluate_tactic_macro_component_sequence(
    pool: &NativeTacticProposalPool,
    candidate: &DiscoveredMacroCandidate,
    frontier: &TacticMacroValidationFrontier,
    encoder: &GoalConditionedTacticFeatureEncoder,
    output_root: &Path,
    accounting: &mut TacticMacroValidationAccounting,
) -> Result<TacticMacroMeasuredOutcome, NativeTacticRouteRunError> {
    let before_distance =
        encoder.encode(&frontier.snapshot).map_err(route_error)?[encoder.goal_distance_feature()];
    let mut snapshot = frontier.snapshot.clone();
    let mut route_tape = frontier.route_tape.clone();
    let mut terminal = false;
    let mut ticks = 0_u32;
    for (component_index, component) in candidate.components.iter().enumerate() {
        if terminal {
            break;
        }
        let entry = component.catalog_entry().map_err(route_error)?;
        let catalog = Arc::new(TacticAssetCatalog::new(vec![entry]).map_err(route_error)?);
        let selected = SelectedTactic {
            schema: TACTIC_EXPLORATION_SCHEMA_V1.into(),
            learner_snapshot_sha256: snapshot.content_sha256().map_err(route_error)?,
            decision_index: component_index as u64,
            descriptor: component.action.clone(),
            reason: TacticSelectionReason::StructuredBaseline,
            exploration_draw: 0,
        };
        let component_root = output_root.join(format!("component-{component_index:03}"));
        let mut work = pool.execute_batch(
            std::slice::from_ref(&selected),
            catalog,
            Arc::new(Vec::new()),
            &snapshot,
            &route_tape,
            None,
            None,
            false,
            &component_root,
        )?;
        if work.len() != 1 {
            return Err(route_message(
                "macro primitive-component execution did not produce one outcome",
            ));
        }
        let evaluated = work.remove(0);
        merge_tactic_macro_validation_accounting(accounting, &evaluated);
        if evaluated.outcome.selected.descriptor != component.action
            || evaluated.outcome.route_tape.frames.len() < route_tape.frames.len()
            || evaluated.outcome.route_tape.frames[..route_tape.frames.len()] != route_tape.frames
        {
            return Err(route_message(
                "macro primitive-component outcome is detached from its source sequence",
            ));
        }
        ticks = ticks
            .checked_add(evaluated.outcome.execution.duration.realized_ticks)
            .ok_or_else(|| route_message("macro primitive-component ticks overflow"))?;
        terminal = evaluated.outcome.terminal;
        snapshot = evaluated.outcome.next_facts;
        route_tape = evaluated.outcome.route_tape;
    }
    let after_distance =
        encoder.encode(&snapshot).map_err(route_error)?[encoder.goal_distance_feature()];
    Ok(TacticMacroMeasuredOutcome {
        terminal,
        progress: before_distance - after_distance,
        ticks,
        emitted_raw_actions: route_tape.frames[frontier.route_tape.frames.len()..].to_vec(),
    })
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
        None,
        false,
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
    accounting.ipc_and_result_transport_micros = accounting
        .ipc_and_result_transport_micros
        .saturating_add(elapsed_micros(evaluated.ipc_elapsed));
    accounting.native_observation_capture_micros = accounting
        .native_observation_capture_micros
        .saturating_add(elapsed_micros(evaluated.observation_capture_elapsed));
    accounting.native_corpus_encoding_micros = accounting
        .native_corpus_encoding_micros
        .saturating_add(elapsed_micros(evaluated.corpus_encoding_elapsed));
    accounting.rust_state_extraction_micros = accounting
        .rust_state_extraction_micros
        .saturating_add(evaluated.outcome.state_extraction_micros);
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
        None,
        false,
        output_root,
    )?;
    let before_distance =
        encoder.encode(&frontier.snapshot).map_err(route_error)?[encoder.goal_distance_feature()];
    work.into_iter()
        .map(|evaluated| {
            merge_tactic_macro_validation_accounting(accounting, &evaluated);
            let ticks = evaluated.outcome.execution.duration.realized_ticks;
            let after_distance = encoder
                .encode(&evaluated.outcome.next_facts)
                .map_err(route_error)?[encoder.goal_distance_feature()];
            Ok((
                evaluated.outcome.selected.descriptor.option_id,
                TacticMacroMeasuredOutcome {
                    terminal: evaluated.outcome.terminal,
                    progress: before_distance - after_distance,
                    ticks,
                    emitted_raw_actions: evaluated.outcome.execution.emitted_raw_actions,
                },
            ))
        })
        .collect()
}

fn merge_tactic_macro_validation_accounting(
    accounting: &mut TacticMacroValidationAccounting,
    evaluated: &NativeTacticProposalWork,
) {
    accounting.native_simulation_micros = accounting
        .native_simulation_micros
        .saturating_add(elapsed_micros(evaluated.native_elapsed));
    accounting.ipc_and_result_transport_micros = accounting
        .ipc_and_result_transport_micros
        .saturating_add(elapsed_micros(evaluated.ipc_elapsed));
    accounting.native_observation_capture_micros = accounting
        .native_observation_capture_micros
        .saturating_add(elapsed_micros(evaluated.observation_capture_elapsed));
    accounting.native_corpus_encoding_micros = accounting
        .native_corpus_encoding_micros
        .saturating_add(elapsed_micros(evaluated.corpus_encoding_elapsed));
    accounting.rust_state_extraction_micros = accounting
        .rust_state_extraction_micros
        .saturating_add(evaluated.outcome.state_extraction_micros);
    accounting.preparation_micros = accounting
        .preparation_micros
        .saturating_add(elapsed_micros(evaluated.preparation_elapsed));
    accounting.restore.merge(&evaluated.restore_accounting);
    accounting.native_ticks = accounting.native_ticks.saturating_add(u64::from(
        evaluated.outcome.execution.duration.realized_ticks,
    ));
}

pub(super) fn collect_tactic_macro_validation_frontiers(
    output_root: &Path,
    source_lanes: &[TacticMacroSourceLane],
    root_checkpoint_sha256: Digest,
) -> Result<Vec<TacticMacroValidationFrontier>, NativeTacticRouteRunError> {
    let mut frontiers = BTreeMap::new();
    for source_lane in source_lanes {
        let seed_index = source_lane.seed_index;
        let seed = source_lane.seed;
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
            }
            if first_transition.before.terminal.reached != Some(true) {
                insert_tactic_macro_validation_frontier(
                    &mut frontiers,
                    TacticMacroValidationFrontier {
                        seed,
                        state_sha256: first_transition.before_state_sha256,
                        snapshot: first_transition.before,
                        route_tape: source_route,
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
        Some(_) => Ok(()),
        None => {
            frontiers.insert(identity, frontier);
            Ok(())
        }
    }
}
