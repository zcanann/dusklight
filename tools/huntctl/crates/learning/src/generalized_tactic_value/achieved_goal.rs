use super::reverse_costs::ReverseCostGraph;
use super::*;
use crate::tactic_features::TacticFeatureEncoder;
use std::collections::BTreeMap;

const MAX_ACHIEVED_GOAL_TARGETS: usize = 32;

fn achieved_goal_target_budget(transition_count: usize, remaining_samples: usize) -> usize {
    if transition_count == 0 {
        return 0;
    }
    (remaining_samples / transition_count)
        .min(MAX_ACHIEVED_GOAL_TARGETS)
        .min(transition_count.isqrt().max(1))
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
    fit_inner(transitions, goal_distance_feature, false)
}

pub(super) fn fit_delayed(
    transitions: &[OptionTransitionSample],
    goal_distance_feature: usize,
) -> Result<GeneralizedTacticValueModel, GeneralizedTacticValueError> {
    fit_inner(transitions, goal_distance_feature, true)
}

fn fit_inner(
    transitions: &[OptionTransitionSample],
    goal_distance_feature: usize,
    delayed_returns: bool,
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
    let graph = reverse_graph(&edges);
    let native_returns = if delayed_returns {
        authenticated_terminal_conditional_returns(transitions)?
    } else {
        vec![None; transitions.len()]
    };
    // The motion control retains direct outcomes with no return authority.
    // The delayed treatment supplies a closed native return where available,
    // and otherwise learns from the action's own achieved endpoint.
    let mut samples = transitions
        .iter()
        .enumerate()
        .map(|(index, transition)| {
            let mut outcome =
                GeneralizedTacticOutcome::from_transition(transition, goal_distance_feature)?;
            outcome.reward = 0.0;
            outcome.terminal = 0.0;
            let mut state_features = if let Some(value) = native_returns[index] {
                // Native goal returns take precedence for the authored query.
                // Other, unfinished paths still contribute hindsight tasks.
                outcome.reward = value;
                outcome.terminal = 1.0;
                outcome.duration_ticks = -value;
                transition.value_sample.state.clone()
            } else if delayed_returns {
                // Every observed action reached its own endpoint, including
                // actions from otherwise unfinished episodes. This is a real
                // short-horizon hindsight label, not a failure or zero return
                // for some other goal.
                let encoder = GoalConditionedTacticFeatureEncoder::new(
                    transition
                        .after
                        .player
                        .position_f32_bits
                        .map(f32::from_bits),
                )
                .map_err(|error| GeneralizedTacticValueError::InvalidFacts(error.to_string()))?;
                let features = encoder
                    .encode_from_base(&transition.before, &base_features[index].0)
                    .map_err(|error| {
                        GeneralizedTacticValueError::InvalidFacts(error.to_string())
                    })?;
                outcome.reward = -(transition.value_sample.duration_ticks as f32);
                outcome.goal_progress_per_tick =
                    features[goal_distance_feature] / transition.value_sample.duration_ticks as f32;
                features
            } else {
                transition.value_sample.state.clone()
            };
            if delayed_returns {
                state_features.push(if native_returns[index].is_some() {
                    GoalQueryKind::Authored.feature()
                } else {
                    GoalQueryKind::Achieved.feature()
                });
            }
            Ok(GeneralizedTacticTrainingSample {
                state_features,
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
        sampled_targets(transitions, maximum_targets, delayed_returns)
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
        let costs = reverse_path_costs(&edges, &graph, target_transition.after_state_sha256);
        for (transition_index, transition) in transitions.iter().enumerate() {
            if samples.len() == MAX_GENERALIZED_TACTIC_SAMPLES {
                break 'targets;
            }
            if delayed_returns
                && (transition.after_state_sha256 == target_transition.after_state_sha256
                    || !costs.contains_key(&transition_index))
            {
                // Own endpoints were inserted above. Unconnected rows do not
                // acquire a fabricated zero-cost continuation to this goal.
                continue;
            }
            let (before_base, after_base) = &base_features[transition_index];
            let mut state_features = encoder
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
            if delayed_returns {
                state_features.push(GoalQueryKind::Achieved.feature());
            }
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
    let mut weights = reference.distance_weights();
    if delayed_returns {
        weights.push(1.0);
    }
    let mut model =
        GeneralizedTacticValueModel::fit_with_state_distance_weights(&samples, &weights)?;
    if delayed_returns {
        model.goal_query_kind = Some(if native_returns.iter().any(Option::is_some) {
            GoalQueryKind::Authored
        } else {
            GoalQueryKind::Achieved
        });
    }
    // Achieved-goal labels are integral native ticks. Differences below half
    // a tick are interpolation noise, not evidence of a faster action.
    model.return_comparison_resolution = 0.5;
    Ok(model)
}

fn reverse_graph(edges: &[ReverseEdge]) -> ReverseCostGraph {
    ReverseCostGraph::new(
        edges
            .iter()
            .map(|edge| (edge.before, edge.after, edge.ticks)),
    )
}

fn sampled_targets(
    transitions: &[OptionTransitionSample],
    maximum_targets: usize,
    prefer_endpoints: bool,
) -> Vec<usize> {
    debug_assert!(maximum_targets > 0);
    let mut unique = BTreeMap::<Digest, usize>::new();
    for (index, transition) in transitions.iter().enumerate() {
        unique.entry(transition.after_state_sha256).or_insert(index);
    }
    if prefer_endpoints {
        // Retain complete recorded continuations as hindsight tasks before
        // filling the remaining budget with interior goals. No authored goal
        // or route identifier participates in this sampling policy.
        let sources = transitions
            .iter()
            .map(|row| row.before_state_sha256)
            .collect::<BTreeSet<_>>();
        let endpoints = unique
            .iter()
            .filter(|(state, _)| !sources.contains(state))
            .map(|(_, index)| *index)
            .collect::<Vec<_>>();
        let stride = endpoints.len().div_ceil(maximum_targets).max(1);
        let mut selected = endpoints
            .into_iter()
            .step_by(stride)
            .take(maximum_targets)
            .collect::<Vec<_>>();
        for index in unique.values().copied() {
            if selected.len() == maximum_targets {
                break;
            }
            if !selected.contains(&index) {
                selected.push(index);
            }
        }
        return selected;
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
    graph: &ReverseCostGraph,
    target: Digest,
) -> BTreeMap<usize, u64> {
    let states = graph.costs([(target, 0)]);
    edges
        .iter()
        .filter_map(|edge| {
            states.get(&edge.after).map(|ticks| {
                (
                    edge.transition_index,
                    ticks.saturating_add(u64::from(edge.ticks)),
                )
            })
        })
        .collect()
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
        let costs = reverse_path_costs(&edges, &reverse_graph(&edges), Digest([3; 32]));

        assert_eq!(costs[&0], 12);
        assert_eq!(costs[&1], 8);
        assert_eq!(costs[&2], 80);
        assert_eq!(costs[&3], 40);
    }
}
