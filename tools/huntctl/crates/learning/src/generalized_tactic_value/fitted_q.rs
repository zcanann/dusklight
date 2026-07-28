use super::*;

pub(super) struct FittedQResult {
    pub values: Vec<f32>,
    pub exact_terminal_supported: BTreeSet<usize>,
    pub exact_first_hit_ticks: Vec<Option<u64>>,
}

pub(super) fn fit_transition_returns(
    transitions: &[OptionTransitionSample],
    minimum_iterations: usize,
    per_tick_discount: f32,
) -> Result<FittedQResult, GeneralizedTacticValueError> {
    let successors = successor_action_indices(transitions)?;
    let immediate = transitions
        .iter()
        .map(|transition| transition.value_sample.reward)
        .collect::<Vec<_>>();
    let durations = transitions
        .iter()
        .map(|transition| transition.value_sample.duration_ticks)
        .collect::<Vec<_>>();
    let terminal = transitions
        .iter()
        .map(|transition| transition.value_sample.terminal)
        .collect::<Vec<_>>();
    let backup_limit = fitted_q_backup_limit(minimum_iterations, transitions.len());
    let values = bellman_returns(
        &immediate,
        &durations,
        &terminal,
        &successors,
        backup_limit,
        minimum_iterations,
        per_tick_discount,
    )?;
    let exact_terminal_supported = terminal_supported_transition_indices(transitions);
    let exact_first_hit_ticks =
        terminal_supported_first_hit_ticks(transitions, &exact_terminal_supported, backup_limit)?;
    Ok(FittedQResult {
        values,
        exact_terminal_supported,
        exact_first_hit_ticks,
    })
}

fn successor_action_indices(
    transitions: &[OptionTransitionSample],
) -> Result<Vec<Vec<usize>>, GeneralizedTacticValueError> {
    let mut exact = BTreeMap::<Digest, Vec<usize>>::new();
    for (index, transition) in transitions.iter().enumerate() {
        exact
            .entry(transition.before_state_sha256)
            .or_default()
            .push(index);
    }
    let before_contexts = transitions
        .iter()
        .map(|transition| Ok(GeneralizedTacticContext::from_facts(&transition.before)?.values()))
        .collect::<Result<Vec<_>, _>>()?;
    let after_contexts = transitions
        .iter()
        .map(|transition| Ok(GeneralizedTacticContext::from_facts(&transition.after)?.values()))
        .collect::<Result<Vec<_>, _>>()?;
    let (minimum, range) = fixed_feature_ranges(
        before_contexts
            .iter()
            .chain(&after_contexts)
            .map(|context| context),
    );

    transitions
        .iter()
        .enumerate()
        .map(|(index, transition)| {
            if transition.value_sample.terminal {
                return Ok(Vec::new());
            }
            if let Some(successors) = exact.get(&transition.after_state_sha256) {
                return Ok(successors.clone());
            }
            let after = &after_contexts[index];
            let nearest_distance = transitions
                .iter()
                .enumerate()
                .filter(|(candidate, successor)| {
                    *candidate != index && approximately_compatible(transition, successor)
                })
                .map(|(candidate, _)| {
                    normalized_distance(after, &before_contexts[candidate], &minimum, &range)
                })
                .min_by(f32::total_cmp);
            Ok(nearest_distance.map_or_else(Vec::new, |nearest_distance| {
                transitions
                    .iter()
                    .enumerate()
                    .filter(|(candidate, successor)| {
                        *candidate != index && approximately_compatible(transition, successor)
                    })
                    .filter_map(|(candidate, _)| {
                        let distance = normalized_distance(
                            after,
                            &before_contexts[candidate],
                            &minimum,
                            &range,
                        );
                        (distance <= nearest_distance + EXACT_STATE_DISTANCE_EPSILON)
                            .then_some(candidate)
                    })
                    .collect()
            }))
        })
        .collect()
}

fn approximately_compatible(
    transition: &OptionTransitionSample,
    successor: &OptionTransitionSample,
) -> bool {
    let after = &transition.after;
    let before = &successor.before;
    after.world.stage == before.world.stage
        && after.world.room == before.world.room
        && after.world.layer == before.world.layer
        && after.player.present == before.player.present
        && after.player.is_link == before.player.is_link
        && after.player.procedure == before.player.procedure
        && after.player.mode_flags == before.player.mode_flags
        && after.player.action_lanes == before.player.action_lanes
        && after
            .player
            .action_state
            .map(|action| (action.do_status, action.flags))
            == before
                .player
                .action_state
                .map(|action| (action.do_status, action.flags))
        && after.event == before.event
        && after.channels.player_action == before.channels.player_action
        && after.terminal.configured == before.terminal.configured
        && before.terminal.reached == Some(false)
}

#[allow(clippy::too_many_arguments)]
fn bellman_returns(
    immediate: &[f32],
    durations: &[u32],
    terminal: &[bool],
    successors: &[Vec<usize>],
    backup_limit: usize,
    minimum_iterations: usize,
    per_tick_discount: f32,
) -> Result<Vec<f32>, GeneralizedTacticValueError> {
    if immediate.len() != durations.len()
        || immediate.len() != terminal.len()
        || immediate.len() != successors.len()
    {
        return Err(GeneralizedTacticValueError::InvalidTransition(
            "fitted-Q arrays have different lengths".into(),
        ));
    }
    let mut values = immediate.to_vec();
    for iteration in 0..backup_limit {
        let prior = values.clone();
        let mut changed = false;
        for index in 0..values.len() {
            values[index] = if terminal[index] || successors[index].is_empty() {
                immediate[index]
            } else {
                let next_value = successors[index]
                    .iter()
                    .map(|next| prior.get(*next).copied())
                    .collect::<Option<Vec<_>>>()
                    .and_then(|values| values.into_iter().max_by(f32::total_cmp))
                    .ok_or_else(|| {
                        GeneralizedTacticValueError::InvalidTransition(
                            "fitted-Q successor index is detached".into(),
                        )
                    })?;
                let duration = i32::try_from(durations[index]).map_err(|_| {
                    GeneralizedTacticValueError::InvalidTransition(
                        "action duration exceeds fitted-Q discount bounds".into(),
                    )
                })?;
                immediate[index] + per_tick_discount.powi(duration) * next_value
            };
            if !values[index].is_finite() {
                return Err(GeneralizedTacticValueError::NonFinite);
            }
            changed |= (values[index] - prior[index]).abs() > 1.0e-6;
        }
        if iteration + 1 >= minimum_iterations && !changed {
            break;
        }
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approximate_rejoin_propagates_value_without_claiming_an_exact_terminal_edge() {
        // Straight is the greedy local action and reaches a censored dead end.
        // Turning initially costs the same, but its approximate successor is
        // the observed around-corner state that reaches the hidden terminal.
        let immediate = [-1.0, -1.0, 100.0];
        let durations = [1, 1, 1];
        let terminal = [false, false, true];
        let successors = [Vec::new(), vec![2], Vec::new()];
        let values =
            bellman_returns(&immediate, &durations, &terminal, &successors, 8, 2, 0.99).unwrap();

        assert_eq!(values[0], -1.0);
        assert!(values[1] > values[0]);
        assert_eq!(values[2], 100.0);
    }
}
