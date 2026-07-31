use super::*;

pub(super) fn elapsed_micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

pub(super) fn decision_trace_is_useful(decision: &NativeTacticDecisionTrace) -> bool {
    decision.terminal
        || decision.reward > 0.0
        || decision.goal_distance_after < decision.goal_distance_before
}

pub(super) fn decision_evaluated_ticks(decision: &NativeTacticDecisionTrace) -> u64 {
    if decision.proposal_batch.is_empty() {
        u64::from(decision.reward_components.duration_ticks)
    } else {
        decision
            .proposal_batch
            .iter()
            .map(|proposal| u64::from(proposal.realized_ticks))
            .sum()
    }
}

pub(super) fn per_second_millionths(count: u64, wall_micros: u64) -> u64 {
    if count == 0 || wall_micros == 0 {
        return 0;
    }
    let scaled = u128::from(count)
        .saturating_mul(1_000_000)
        .saturating_mul(1_000_000)
        / u128::from(wall_micros);
    u64::try_from(scaled).unwrap_or(u64::MAX)
}

pub(super) fn ratio_per_million(numerator: u64, denominator: u64) -> u64 {
    if denominator == 0 {
        return 0;
    }
    let scaled = u128::from(numerator).saturating_mul(1_000_000) / u128::from(denominator);
    u64::try_from(scaled).unwrap_or(u64::MAX)
}

fn censored_episode_end_count(rows: impl IntoIterator<Item = (u64, bool)>) -> u64 {
    let mut episode_ends = BTreeMap::<u64, bool>::new();
    for (episode_group, terminal) in rows {
        episode_ends.insert(episode_group, terminal);
    }
    episode_ends.values().filter(|terminal| !**terminal).count() as u64
}

pub(super) fn censored_training_transitions(corpus: &TacticQTrainingCorpus) -> u64 {
    censored_episode_end_count(
        corpus
            .transitions
            .iter()
            .zip(&corpus.episode_groups)
            .map(|(transition, episode_group)| (*episode_group, transition.value_sample.terminal)),
    )
}

pub(super) fn useful_training_transitions(
    corpus: &TacticQTrainingCorpus,
    goal_distance_feature: usize,
) -> u64 {
    corpus
        .transitions
        .iter()
        .filter(|transition| {
            transition.value_sample.terminal
                || transition.value_sample.reward > 0.0
                || transition
                    .value_sample
                    .state
                    .get(goal_distance_feature)
                    .zip(
                        transition
                            .value_sample
                            .next_state
                            .get(goal_distance_feature),
                    )
                    .is_some_and(|(before, after)| after < before)
        })
        .count() as u64
}

pub(super) fn aggregate_route_timing(
    seeds: &[NativeTacticSeedResult],
    unique_useful_graph_expansions: u64,
) -> NativeTacticRouteTiming {
    let mut timing = NativeTacticRouteTiming::default();
    for seed in seeds {
        timing.wall_micros = timing.wall_micros.saturating_add(seed.timing.wall_micros);
        timing.process_launch_micros = timing
            .process_launch_micros
            .saturating_add(seed.timing.process_launch_micros);
        timing.tactic_selection_micros = timing
            .tactic_selection_micros
            .saturating_add(seed.timing.tactic_selection_micros);
        timing.checkpoint_branching_micros = timing
            .checkpoint_branching_micros
            .saturating_add(seed.timing.checkpoint_branching_micros);
        timing.tactic_execution_micros = timing
            .tactic_execution_micros
            .saturating_add(seed.timing.tactic_execution_micros);
        timing.native_simulation_micros = timing
            .native_simulation_micros
            .saturating_add(seed.timing.native_simulation_micros);
        timing.ipc_and_result_transport_micros = timing
            .ipc_and_result_transport_micros
            .saturating_add(seed.timing.ipc_and_result_transport_micros);
        timing.native_observation_capture_micros = timing
            .native_observation_capture_micros
            .saturating_add(seed.timing.native_observation_capture_micros);
        timing.native_corpus_encoding_micros = timing
            .native_corpus_encoding_micros
            .saturating_add(seed.timing.native_corpus_encoding_micros);
        timing.rust_state_extraction_micros = timing
            .rust_state_extraction_micros
            .saturating_add(seed.timing.rust_state_extraction_micros);
        timing.tactic_preparation_and_fact_extraction_micros = timing
            .tactic_preparation_and_fact_extraction_micros
            .saturating_add(seed.timing.tactic_preparation_and_fact_extraction_micros);
        timing.model_update_micros = timing
            .model_update_micros
            .saturating_add(seed.timing.model_update_micros);
        timing.evidence_projection_and_persistence_micros = timing
            .evidence_projection_and_persistence_micros
            .saturating_add(seed.timing.evidence_projection_and_persistence_micros);
        timing.evidence_projection_micros = timing
            .evidence_projection_micros
            .saturating_add(seed.timing.evidence_projection_micros);
        timing.persistence_micros = timing
            .persistence_micros
            .saturating_add(seed.timing.persistence_micros);
        if let Some(persistence_breakdown) = seed.timing.persistence_breakdown {
            timing
                .persistence_breakdown
                .get_or_insert_default()
                .merge(persistence_breakdown);
        } else if seed.timing.persistence_micros > 0 {
            let persistence_breakdown = timing.persistence_breakdown.get_or_insert_default();
            persistence_breakdown.unattributed_micros = persistence_breakdown
                .unattributed_micros
                .saturating_add(seed.timing.persistence_micros);
        }
        timing.orchestration_micros = timing
            .orchestration_micros
            .saturating_add(seed.timing.orchestration_micros);
        timing.result_validation_and_fact_extraction_micros = timing
            .result_validation_and_fact_extraction_micros
            .saturating_add(seed.timing.result_validation_and_fact_extraction_micros);
        timing.campaign_admission_micros = timing
            .campaign_admission_micros
            .saturating_add(seed.timing.campaign_admission_micros);
        if let Some(admission_breakdown) = seed.timing.campaign_admission_breakdown {
            timing
                .campaign_admission_breakdown
                .get_or_insert_default()
                .merge(admission_breakdown);
        }
        timing.graph_admission_micros = timing
            .graph_admission_micros
            .saturating_add(seed.timing.graph_admission_micros);
        timing.retained_candidate_artifact_micros = timing
            .retained_candidate_artifact_micros
            .saturating_add(seed.timing.retained_candidate_artifact_micros);
        timing.reporting_micros = timing
            .reporting_micros
            .saturating_add(seed.timing.reporting_micros);
    }
    refresh_route_throughput(&mut timing, seeds, unique_useful_graph_expansions);
    timing
}

pub(super) fn record_persistence_timing(
    timing: &mut NativeTacticRouteTiming,
    breakdown: NativeTacticPersistenceTiming,
) {
    let persistence_micros = breakdown.total_micros();
    timing.persistence_micros = timing.persistence_micros.saturating_add(persistence_micros);
    timing.evidence_projection_and_persistence_micros = timing
        .evidence_projection_and_persistence_micros
        .saturating_add(persistence_micros);
    timing
        .persistence_breakdown
        .get_or_insert_default()
        .merge(breakdown);
}

pub(super) fn accumulated_coordinator_wall_micros(
    execution_plan: &NativeTacticExecutionPlan,
    seeds: &[NativeTacticSeedResult],
) -> u64 {
    let seed_wall_micros = seeds
        .iter()
        .map(|seed| seed.timing.wall_micros)
        .collect::<Vec<_>>();
    accumulated_parallel_wall_micros(
        execution_plan
            .generations
            .iter()
            .map(|generation| generation.lane_indices.as_slice()),
        &seed_wall_micros,
    )
}

fn accumulated_parallel_wall_micros<'a>(
    generation_lane_indices: impl IntoIterator<Item = &'a [usize]>,
    seed_wall_micros: &[u64],
) -> u64 {
    generation_lane_indices
        .into_iter()
        .map(|lane_indices| {
            lane_indices
                .iter()
                .filter_map(|lane_index| seed_wall_micros.get(*lane_index))
                .copied()
                .max()
                .unwrap_or(0)
        })
        .fold(0_u64, u64::saturating_add)
}

pub(super) fn refresh_route_throughput(
    timing: &mut NativeTacticRouteTiming,
    seeds: &[NativeTacticSeedResult],
    unique_useful_graph_expansions: u64,
) {
    let useful_decisions = seeds.iter().map(|seed| seed.useful_decisions).sum();
    let native_ticks = seeds.iter().map(|seed| seed.native_ticks).sum();
    let episodes = seeds.iter().map(|seed| seed.episodes).sum();
    timing.useful_decisions_per_second_millionths =
        per_second_millionths(useful_decisions, timing.wall_micros);
    timing.unique_useful_graph_expansions_per_second_millionths =
        per_second_millionths(unique_useful_graph_expansions, timing.wall_micros);
    timing.native_ticks_per_second_millionths =
        per_second_millionths(native_ticks, timing.wall_micros);
    timing.episodes_per_second_millionths = per_second_millionths(episodes, timing.wall_micros);
}

#[cfg(test)]
mod tests {
    use super::{accumulated_parallel_wall_micros, censored_episode_end_count};

    #[test]
    fn only_non_terminal_episode_ends_are_censored() {
        assert_eq!(
            censored_episode_end_count([(10, false), (10, true), (20, false), (30, false)]),
            2
        );
    }

    #[test]
    fn accumulated_wall_sums_generation_critical_paths_without_summing_parallel_lanes() {
        let generation_zero = [0, 1];
        let generation_one = [2];
        assert_eq!(
            accumulated_parallel_wall_micros(
                [generation_zero.as_slice(), generation_one.as_slice()],
                &[100, 150, 75],
            ),
            225
        );
    }
}
