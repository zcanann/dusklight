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

pub(super) fn aggregate_route_timing(seeds: &[NativeTacticSeedResult]) -> NativeTacticRouteTiming {
    let mut timing = NativeTacticRouteTiming::default();
    for seed in seeds {
        timing.wall_micros = timing.wall_micros.saturating_add(seed.timing.wall_micros);
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
        timing.tactic_preparation_and_fact_extraction_micros = timing
            .tactic_preparation_and_fact_extraction_micros
            .saturating_add(seed.timing.tactic_preparation_and_fact_extraction_micros);
        timing.model_update_micros = timing
            .model_update_micros
            .saturating_add(seed.timing.model_update_micros);
        timing.evidence_projection_and_persistence_micros = timing
            .evidence_projection_and_persistence_micros
            .saturating_add(seed.timing.evidence_projection_and_persistence_micros);
        timing.retained_candidate_artifact_micros = timing
            .retained_candidate_artifact_micros
            .saturating_add(seed.timing.retained_candidate_artifact_micros);
    }
    refresh_route_throughput(&mut timing, seeds);
    timing
}

pub(super) fn refresh_route_throughput(
    timing: &mut NativeTacticRouteTiming,
    seeds: &[NativeTacticSeedResult],
) {
    let useful_decisions = seeds.iter().map(|seed| seed.useful_decisions).sum();
    let native_ticks = seeds.iter().map(|seed| seed.native_ticks).sum();
    let episodes = seeds.iter().map(|seed| seed.episodes).sum();
    timing.useful_decisions_per_second_millionths =
        per_second_millionths(useful_decisions, timing.wall_micros);
    timing.native_ticks_per_second_millionths =
        per_second_millionths(native_ticks, timing.wall_micros);
    timing.episodes_per_second_millionths = per_second_millionths(episodes, timing.wall_micros);
}
