use super::*;
use super::macro_discovery::{
    compare_tactic_macro_candidate_priority, macro_entry_observation,
    tactic_macro_component_for_transition,
};

const MAX_TERMINAL_LINEAGE_OCCURRENCES: usize = MAX_DISCOVERY_OBSERVATIONS;
const MAX_SINGLE_SOURCE_TERMINAL_LINEAGE_CANDIDATES: usize = 8;

pub(super) type ConnectedMacroOccurrences = BTreeMap<
    Vec<u8>,
    (
        InputTape,
        Vec<TacticMacroComponent>,
        BTreeMap<Vec<Digest>, MacroSourceProvenance>,
    ),
>;

#[derive(Clone)]
struct TerminalLineageEdge {
    seed: u64,
    source_checkpoint_sha256: Digest,
    next_checkpoint_sha256: Digest,
    before_state_sha256: Digest,
    after_state_sha256: Digest,
    source_route: InputTape,
    endpoint_route: InputTape,
    component: TacticMacroComponent,
    transition_sha256: Digest,
    entry: MacroEntryObservation,
    terminal: bool,
}

pub(super) fn mine_terminal_lineage_tactic_macro_compositions(
    output_root: &Path,
    source_lanes: &[TacticMacroSourceLane],
    encoder: &GoalConditionedTacticFeatureEncoder,
) -> Result<Vec<DiscoveredMacroCandidate>, NativeTacticRouteRunError> {
    let mut edges = Vec::new();
    for source_lane in source_lanes {
        let seed_root = output_root.join(format!(
            "seed-{:03}-{}",
            source_lane.seed_index, source_lane.seed
        ));
        let replay = load_tactic_journal_replay(&seed_root)?;
        let store = TacticQContentStore::open(tactic_content_store_path(&seed_root))
            .map_err(route_error)?;
        for (index, record) in replay.records.iter().enumerate() {
            let selected_route = replay
                .routes
                .get(index)
                .ok_or_else(|| route_message("macro lineage selected route is absent"))?;
            let first_transition = replay
                .transitions
                .get(index)
                .ok_or_else(|| route_message("macro lineage selected transition is absent"))?;
            let source_frame = usize::try_from(
                first_transition.execution.realized_tape_range.start_frame,
            )
            .map_err(route_error)?;
            if source_frame > selected_route.frames.len() {
                return Err(route_message(
                    "macro lineage source frame exceeds its authenticated route",
                ));
            }
            let mut source_route = selected_route.clone();
            source_route.frames.truncate(source_frame);
            source_route.validate().map_err(route_error)?;

            if record.proposal_batch.is_empty() {
                collect_terminal_lineage_edge(
                    &mut edges,
                    source_lane.seed,
                    record.decision_index,
                    replay.root_checkpoint_sha256,
                    encoder,
                    first_transition.clone(),
                    journal_transition_sha256(
                        record.transition,
                        record.inline_transition.as_ref(),
                    )?,
                    None,
                    &source_route,
                )?;
            } else {
                for proposal in &record.proposal_batch {
                    let transition = journal_transition(
                        &store,
                        proposal.transition,
                        proposal.inline_transition.as_ref(),
                    )?;
                    collect_terminal_lineage_edge(
                        &mut edges,
                        source_lane.seed,
                        record.decision_index,
                        replay.root_checkpoint_sha256,
                        encoder,
                        transition,
                        journal_transition_sha256(
                            proposal.transition,
                            proposal.inline_transition.as_ref(),
                        )?,
                        proposal.component.as_ref(),
                        &source_route,
                    )?;
                }
            }
        }
    }
    let occurrences = terminal_lineage_occurrences(&edges)?;
    terminal_lineage_macro_candidates(occurrences)
}

#[allow(clippy::too_many_arguments)]
fn collect_terminal_lineage_edge(
    edges: &mut Vec<TerminalLineageEdge>,
    seed: u64,
    decision_index: u64,
    root_checkpoint_sha256: Digest,
    encoder: &GoalConditionedTacticFeatureEncoder,
    transition: OptionTransitionSample,
    transition_sha256: Digest,
    retained_component: Option<&TacticMacroComponent>,
    source_route: &InputTape,
) -> Result<(), NativeTacticRouteRunError> {
    if !transition.value_sample.action.option_id.starts_with("family/") {
        return Ok(());
    }
    if source_route.frames.len() as u64
        != transition.execution.realized_tape_range.start_frame
        || route_checkpoint(root_checkpoint_sha256, source_route).map_err(route_error)?
            != transition.source_checkpoint_sha256
    {
        return Err(route_message(
            "macro lineage edge is detached from its authenticated source route",
        ));
    }
    let mut endpoint_route = source_route.clone();
    endpoint_route
        .frames
        .extend_from_slice(&transition.execution.emitted_raw_actions);
    endpoint_route.validate().map_err(route_error)?;
    if endpoint_route.frames.len() as u64
        != transition.execution.realized_tape_range.end_frame_exclusive
        || route_checkpoint(root_checkpoint_sha256, &endpoint_route).map_err(route_error)?
            != transition.next_checkpoint_sha256
    {
        return Err(route_message(
            "macro lineage edge is detached from its authenticated endpoint route",
        ));
    }
    let component = tactic_macro_component_for_transition(
        seed,
        decision_index,
        &transition,
        encoder,
        retained_component,
    )?;
    edges.push(TerminalLineageEdge {
        seed,
        source_checkpoint_sha256: transition.source_checkpoint_sha256,
        next_checkpoint_sha256: transition.next_checkpoint_sha256,
        before_state_sha256: transition.before_state_sha256,
        after_state_sha256: transition.after_state_sha256,
        source_route: source_route.clone(),
        endpoint_route,
        component,
        transition_sha256,
        entry: macro_entry_observation(&transition.before, encoder)?,
        terminal: transition.value_sample.terminal,
    });
    Ok(())
}

fn terminal_lineage_occurrences(
    edges: &[TerminalLineageEdge],
) -> Result<ConnectedMacroOccurrences, NativeTacticRouteRunError> {
    let mut terminal_routes = BTreeMap::<(u64, Digest), InputTape>::new();
    for edge in edges.iter().filter(|edge| edge.terminal) {
        let encoded = edge.endpoint_route.encode().map_err(route_error)?;
        terminal_routes.insert(
            (edge.seed, Digest(Sha256::digest(&encoded).into())),
            edge.endpoint_route.clone(),
        );
    }
    let mut occurrences = ConnectedMacroOccurrences::new();
    let mut admitted = 0_usize;
    for ((seed, _), terminal_route) in terminal_routes {
        let mut matching = edges
            .iter()
            .filter(|edge| {
                edge.seed == seed
                    && route_is_prefix(&edge.source_route, &terminal_route)
                    && route_is_prefix(&edge.endpoint_route, &terminal_route)
            })
            .collect::<Vec<_>>();
        matching.sort_by(|left, right| {
            left.source_route
                .frames
                .len()
                .cmp(&right.source_route.frames.len())
                .then_with(|| {
                    left.endpoint_route
                        .frames
                        .len()
                        .cmp(&right.endpoint_route.frames.len())
                })
                .then_with(|| left.transition_sha256.cmp(&right.transition_sha256))
        });
        let mut by_start = BTreeMap::<usize, Vec<&TerminalLineageEdge>>::new();
        for edge in matching {
            by_start
                .entry(edge.source_route.frames.len())
                .or_default()
                .push(edge);
        }
        for first in by_start.values().flatten().copied() {
            let mut path = vec![first];
            extend_terminal_lineage_path(
                &mut path,
                &by_start,
                &terminal_route,
                &mut occurrences,
                &mut admitted,
            )?;
            if admitted >= MAX_TERMINAL_LINEAGE_OCCURRENCES {
                break;
            }
        }
        if admitted >= MAX_TERMINAL_LINEAGE_OCCURRENCES {
            break;
        }
    }
    Ok(occurrences)
}

fn extend_terminal_lineage_path<'a>(
    path: &mut Vec<&'a TerminalLineageEdge>,
    by_start: &BTreeMap<usize, Vec<&'a TerminalLineageEdge>>,
    terminal_route: &InputTape,
    occurrences: &mut ConnectedMacroOccurrences,
    admitted: &mut usize,
) -> Result<(), NativeTacticRouteRunError> {
    let first = path[0];
    let last = *path.last().expect("terminal lineage path is nonempty");
    let start = first.source_route.frames.len();
    let end = last.endpoint_route.frames.len();
    let ticks = end.saturating_sub(start);
    if path.len() >= 2 {
        let tape = InputTape {
            boot: terminal_route.boot.clone(),
            tick_rate_numerator: terminal_route.tick_rate_numerator,
            tick_rate_denominator: terminal_route.tick_rate_denominator,
            frames: terminal_route.frames[start..end].to_vec(),
        };
        let components = path
            .iter()
            .map(|edge| edge.component.clone())
            .collect::<Vec<_>>();
        let source = MacroSourceProvenance {
            seed: first.seed,
            frontier_state_sha256: first.before_state_sha256,
            transition_sha256s: path
                .iter()
                .map(|edge| edge.transition_sha256)
                .collect(),
            entry: first.entry.clone(),
        };
        let key = connected_macro_occurrence_key(&tape, &components)?;
        occurrences
            .entry(key)
            .or_insert_with(|| (tape, components, BTreeMap::new()))
            .2
            .insert(source.transition_sha256s.clone(), source);
        *admitted = admitted.saturating_add(1);
        if *admitted >= MAX_TERMINAL_LINEAGE_OCCURRENCES {
            return Ok(());
        }
    }
    if ticks >= MAX_DISCOVERED_MACRO_TICKS {
        return Ok(());
    }
    let Some(next_edges) = by_start.get(&end) else {
        return Ok(());
    };
    for next in next_edges {
        if next.source_checkpoint_sha256 != last.next_checkpoint_sha256
            || next.before_state_sha256 != last.after_state_sha256
        {
            continue;
        }
        let next_ticks = next.endpoint_route.frames.len().saturating_sub(start);
        if next_ticks > MAX_DISCOVERED_MACRO_TICKS {
            continue;
        }
        path.push(next);
        extend_terminal_lineage_path(
            path,
            by_start,
            terminal_route,
            occurrences,
            admitted,
        )?;
        path.pop();
        if *admitted >= MAX_TERMINAL_LINEAGE_OCCURRENCES {
            break;
        }
    }
    Ok(())
}

fn route_is_prefix(prefix: &InputTape, route: &InputTape) -> bool {
    prefix.boot == route.boot
        && prefix.tick_rate_numerator == route.tick_rate_numerator
        && prefix.tick_rate_denominator == route.tick_rate_denominator
        && prefix.frames.len() <= route.frames.len()
        && prefix.frames == route.frames[..prefix.frames.len()]
}

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

fn terminal_lineage_macro_candidates(
    occurrences: ConnectedMacroOccurrences,
) -> Result<Vec<DiscoveredMacroCandidate>, NativeTacticRouteRunError> {
    let mut repeated = Vec::new();
    let mut single_source = Vec::new();
    for (tape, components, sources) in occurrences.into_values() {
        let distinct_states = sources
            .values()
            .map(|source| source.frontier_state_sha256)
            .collect::<BTreeSet<_>>()
            .len();
        if distinct_states >= MIN_DISCOVERY_OCCURRENCES {
            repeated.push(
                replay_macro_candidate(tape, components, sources.into_values().collect())
                    .map_err(route_error)?,
            );
        } else if components.len() >= 2
            && let Some(source) = sources.into_values().next()
        {
            single_source.push(
                terminal_lineage_macro_candidate(tape, components, source).map_err(route_error)?,
            );
        }
    }
    single_source.sort_by(compare_tactic_macro_candidate_priority);
    single_source.truncate(MAX_SINGLE_SOURCE_TERMINAL_LINEAGE_CANDIDATES);
    repeated.extend(single_source);
    Ok(repeated)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tape(values: &[i8]) -> InputTape {
        InputTape {
            frames: values
                .iter()
                .map(|value| {
                    let mut frame = InputFrame::default();
                    frame.owned_ports = 1;
                    frame.pads[0].stick_x = *value;
                    frame
                })
                .collect(),
            ..InputTape::default()
        }
    }

    fn component(option_id: &str, frames: InputTape) -> TacticMacroComponent {
        TacticMacroComponent::from_catalog_entry(
            &TacticCatalogEntry::new(option_id, TacticAssetSource::RecordedTape(frames)).unwrap(),
        )
        .unwrap()
    }

    fn edge(seed: u64, state: u8, first: bool) -> TerminalLineageEdge {
        let complete = tape(&[10, 10, 10, 10, 20, 20, 20, 20]);
        let (start, end, before, after, option_id) = if first {
            (0, 4, state, state.saturating_add(1), "family/first")
        } else {
            (4, 8, state.saturating_add(1), state.saturating_add(2), "family/second")
        };
        let source_route = InputTape {
            frames: complete.frames[..start].to_vec(),
            ..complete.clone()
        };
        let endpoint_route = InputTape {
            frames: complete.frames[..end].to_vec(),
            ..complete.clone()
        };
        TerminalLineageEdge {
            seed,
            source_checkpoint_sha256: Digest([before; 32]),
            next_checkpoint_sha256: Digest([after; 32]),
            before_state_sha256: Digest([before; 32]),
            after_state_sha256: Digest([after; 32]),
            source_route,
            endpoint_route,
            component: component(
                option_id,
                InputTape {
                    frames: complete.frames[start..end].to_vec(),
                    ..complete.clone()
                },
            ),
            transition_sha256: Digest([before.saturating_add(20); 32]),
            entry: MacroEntryObservation {
                stage: "F_SP103".into(),
                room: 1,
                player_procedure: Some(3),
                player_contacts: Some(1),
                goal_distance_f32_bits: (100.0 + f32::from(state)).to_bits(),
            },
            terminal: !first,
        }
    }

    #[test]
    fn terminal_lineage_mining_joins_nonadjacent_journal_edges_only_after_repetition() {
        let first_a = edge(11, 1, true);
        let second_a = edge(11, 1, false);
        let first_b = edge(13, 4, true);
        let second_b = edge(13, 4, false);
        let once = terminal_lineage_occurrences(&[first_a.clone(), second_a.clone()]).unwrap();
        assert!(connected_macro_candidates(once).unwrap().is_empty());
        let once = terminal_lineage_occurrences(&[first_a.clone(), second_a.clone()]).unwrap();
        let terminal_candidates = terminal_lineage_macro_candidates(once).unwrap();
        assert_eq!(terminal_candidates.len(), 1);
        assert_eq!(terminal_candidates[0].components.len(), 2);

        // The order deliberately interleaves independent rows. Connectivity is
        // recovered from authenticated route/checkpoint identity, not adjacency.
        let repeated =
            terminal_lineage_occurrences(&[first_a, first_b, second_a, second_b]).unwrap();
        let candidates = connected_macro_candidates(repeated).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].components.len(), 2);
        assert_eq!(candidates[0].sources.len(), 2);
        assert_eq!(candidates[0].tape.frames.len(), 8);
    }
}
