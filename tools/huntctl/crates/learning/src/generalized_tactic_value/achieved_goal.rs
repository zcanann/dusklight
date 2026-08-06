use super::*;
use crate::tactic_features::TacticFeatureEncoder;
use std::collections::{BTreeMap, VecDeque};

const MAX_ACHIEVED_GOAL_TARGETS: usize = 32;
const MAX_ACHIEVED_GOAL_HOPS: u32 = 16;
const MAX_REVERSE_STATES_PER_TARGET: usize = 4_096;

fn achieved_goal_target_budget(transition_count: usize, remaining_samples: usize) -> usize {
    if transition_count == 0 {
        return 0;
    }
    (remaining_samples / transition_count)
        .min(MAX_ACHIEVED_GOAL_TARGETS)
        .min(transition_count.isqrt().max(1))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ReverseReach {
    ticks: u64,
    hops: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ReverseEdge {
    transition_index: usize,
    before: Digest,
    after: Digest,
    ticks: u32,
}

pub(super) fn fit(
    transitions: &[OptionTransitionSample],
    goal_distance_feature: usize,
) -> Result<GeneralizedTacticValueModel, GeneralizedTacticValueError> {
    if transitions.len() < 2 || transitions.len() > MAX_GENERALIZED_TACTIC_SAMPLES {
        return Err(GeneralizedTacticValueError::SampleCount);
    }
    for transition in transitions {
        transition
            .validate()
            .map_err(|error| GeneralizedTacticValueError::InvalidTransition(error.to_string()))?;
    }
    let reference = GoalConditionedTacticFeatureEncoder::new([0.0; 3])
        .map_err(|error| GeneralizedTacticValueError::InvalidFacts(error.to_string()))?;
    if goal_distance_feature != reference.goal_distance_feature()
        || transitions.iter().any(|transition| {
            transition.feature_schema_sha256 != reference.schema_sha256
                || transition.value_sample.state.len() != reference.feature_width()
        })
    {
        return Err(GeneralizedTacticValueError::FeatureWidth);
    }

    // Goal relabeling changes only the final target-relative columns. The
    // route-agnostic observation is identical for every sampled target, so
    // compute its actor/flag/history/trajectory projection once per physical
    // boundary instead of once per (transition, target) pair.
    let base_encoder = TacticFeatureEncoder::new();
    let base_features = transitions
        .iter()
        .map(|transition| {
            Ok((
                base_encoder.encode(&transition.before).map_err(|error| {
                    GeneralizedTacticValueError::InvalidFacts(error.to_string())
                })?,
                base_encoder.encode(&transition.after).map_err(|error| {
                    GeneralizedTacticValueError::InvalidFacts(error.to_string())
                })?,
            ))
        })
        .collect::<Result<Vec<_>, GeneralizedTacticValueError>>()?;

    let edges = transitions
        .iter()
        .enumerate()
        .map(|(transition_index, transition)| ReverseEdge {
            transition_index,
            before: transition.before_state_sha256,
            after: transition.after_state_sha256,
            ticks: transition.value_sample.duration_ticks,
        })
        .collect::<Vec<_>>();
    let incoming = incoming_edges(&edges);
    // Preserve every directly observed outcome relative to the authored
    // target encoded in the replay row. These are the highest-locality
    // reachability labels at obstacle contacts and repeated exact states. They
    // remain exploration-only: sparse reward and terminal authority are
    // explicitly removed.
    let mut samples = transitions
        .iter()
        .map(|transition| {
            let mut outcome =
                GeneralizedTacticOutcome::from_transition(transition, goal_distance_feature)?;
            outcome.reward = 0.0;
            outcome.terminal = 0.0;
            Ok(GeneralizedTacticTrainingSample {
                state_features: transition.value_sample.state.clone(),
                context: GeneralizedTacticContext::from_facts(&transition.before)?,
                action: transition.value_sample.action.clone(),
                outcome,
            })
        })
        .collect::<Result<Vec<_>, GeneralizedTacticValueError>>()?;
    // Cross-relabel every observed physical transition against a bounded set
    // of achieved targets. Reverse-path rows alone only show what an action
    // did relative to a goal it eventually reached; they omit the
    // counterexamples needed to distinguish target-directed motion from raw
    // speed. Bound the target count further for very large corpora so fitting
    // remains within the model's deterministic sample budget.
    let remaining_samples = MAX_GENERALIZED_TACTIC_SAMPLES.saturating_sub(samples.len());
    let maximum_targets = achieved_goal_target_budget(transitions.len(), remaining_samples);
    let targets = if maximum_targets == 0 {
        Vec::new()
    } else {
        sampled_targets(transitions, maximum_targets)
    };
    'targets: for target_index in targets {
        let target_transition = &transitions[target_index];
        let target = target_transition
            .after
            .player
            .position_f32_bits
            .map(f32::from_bits);
        if target.iter().any(|value| !value.is_finite()) {
            return Err(GeneralizedTacticValueError::NonFinite);
        }
        let encoder = GoalConditionedTacticFeatureEncoder::new(target)
            .map_err(|error| GeneralizedTacticValueError::InvalidFacts(error.to_string()))?;
        let costs = reverse_path_costs(&edges, &incoming, target_transition.after_state_sha256);
        for (transition_index, transition) in transitions.iter().enumerate() {
            if samples.len() == MAX_GENERALIZED_TACTIC_SAMPLES {
                break 'targets;
            }
            let (before_base, after_base) = &base_features[transition_index];
            let state_features = encoder
                .encode_from_base(&transition.before, before_base)
                .map_err(|error| GeneralizedTacticValueError::InvalidFacts(error.to_string()))?;
            let next_features = encoder
                .encode_from_base(&transition.after, after_base)
                .map_err(|error| GeneralizedTacticValueError::InvalidFacts(error.to_string()))?;
            let duration = transition.value_sample.duration_ticks as f32;
            let mut outcome =
                GeneralizedTacticOutcome::from_transition(transition, goal_distance_feature)?;
            // Only an exact reverse path owns an achieved-goal return. Other
            // cross-relabeled rows train relative reachability and carry no
            // value authority.
            outcome.reward = costs
                .get(&transition_index)
                .map_or(0.0, |ticks_to_goal| -(*ticks_to_goal as f32));
            // These are supervised returns to an achieved coordinate, not
            // evidence that the authored native terminal was reached.
            outcome.terminal = 0.0;
            outcome.goal_progress_per_tick = (state_features[goal_distance_feature]
                - next_features[goal_distance_feature])
                / duration;
            samples.push(GeneralizedTacticTrainingSample {
                state_features,
                context: GeneralizedTacticContext::from_facts(&transition.before)?,
                action: transition.value_sample.action.clone(),
                outcome,
            });
        }
    }
    if samples.len() < 2 {
        return Err(GeneralizedTacticValueError::SampleCount);
    }
    let mut model = GeneralizedTacticValueModel::fit_with_state_distance_weights(
        &samples,
        &reference.distance_weights(),
    )?;
    // Achieved-goal labels are integral native ticks. Differences below half
    // a tick are interpolation noise, not evidence of a faster action.
    model.return_comparison_resolution = 0.5;
    Ok(model)
}

fn incoming_edges(edges: &[ReverseEdge]) -> BTreeMap<Digest, Vec<usize>> {
    let mut incoming = BTreeMap::<Digest, Vec<usize>>::new();
    for (index, edge) in edges.iter().enumerate() {
        incoming.entry(edge.after).or_default().push(index);
    }
    incoming
}

fn sampled_targets(transitions: &[OptionTransitionSample], maximum_targets: usize) -> Vec<usize> {
    debug_assert!(maximum_targets > 0);
    let mut unique = BTreeMap::<Digest, usize>::new();
    for (index, transition) in transitions.iter().enumerate() {
        unique.entry(transition.after_state_sha256).or_insert(index);
    }
    let stride = unique.len().div_ceil(maximum_targets).max(1);
    unique
        .into_values()
        .step_by(stride)
        .take(maximum_targets)
        .collect()
}

fn reverse_path_costs(
    edges: &[ReverseEdge],
    incoming: &BTreeMap<Digest, Vec<usize>>,
    target: Digest,
) -> BTreeMap<usize, u64> {
    let mut states = BTreeMap::from([(target, ReverseReach { ticks: 0, hops: 0 })]);
    let mut queue = VecDeque::from([target]);
    let mut edge_costs = BTreeMap::<usize, u64>::new();
    while let Some(state) = queue.pop_front() {
        let reach = states[&state];
        if reach.hops == MAX_ACHIEVED_GOAL_HOPS {
            continue;
        }
        for edge_index in incoming.get(&state).into_iter().flatten().copied() {
            let edge = edges[edge_index];
            let ticks = reach.ticks.saturating_add(u64::from(edge.ticks));
            edge_costs
                .entry(edge.transition_index)
                .and_modify(|best| *best = (*best).min(ticks))
                .or_insert(ticks);
            let before = edge.before;
            let next = ReverseReach {
                ticks,
                hops: reach.hops + 1,
            };
            let improves = states
                .get(&before)
                .is_none_or(|prior| (next.ticks, next.hops) < (prior.ticks, prior.hops));
            if improves && states.len() < MAX_REVERSE_STATES_PER_TARGET {
                states.insert(before, next);
                queue.push_back(before);
            }
        }
    }
    edge_costs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn online_relabeling_breadth_grows_sublinearly() {
        assert_eq!(achieved_goal_target_budget(0, 1_000), 0);
        assert_eq!(achieved_goal_target_budget(2, 1_000), 1);
        assert_eq!(achieved_goal_target_budget(8, 1_000), 2);
        assert_eq!(achieved_goal_target_budget(16, 1_000), 4);
        assert_eq!(achieved_goal_target_budget(32, 1_000), 5);
        assert_eq!(achieved_goal_target_budget(1_024, usize::MAX), 32);
        assert_eq!(achieved_goal_target_budget(1_024, 2_048), 2);
    }

    fn edge(index: usize, before: u8, after: u8, ticks: u32) -> ReverseEdge {
        ReverseEdge {
            transition_index: index,
            before: Digest([before; 32]),
            after: Digest([after; 32]),
            ticks,
        }
    }

    #[test]
    fn reverse_costs_relabel_exact_ancestors_with_shortest_native_ticks() {
        let edges = vec![
            edge(0, 1, 2, 4),
            edge(1, 2, 3, 8),
            edge(2, 1, 4, 40),
            edge(3, 4, 3, 40),
        ];
        let costs = reverse_path_costs(&edges, &incoming_edges(&edges), Digest([3; 32]));

        assert_eq!(costs[&0], 12);
        assert_eq!(costs[&1], 8);
        assert_eq!(costs[&2], 80);
        assert_eq!(costs[&3], 40);
    }
}
