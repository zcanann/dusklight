use super::*;

pub(crate) fn regression_features(
    state_features: &[f32],
    context: &GeneralizedTacticContext,
    descriptor: &OptionActionDescriptor,
) -> Result<Vec<f32>, GeneralizedTacticValueError> {
    if state_features.is_empty() || state_features.iter().any(|value| !value.is_finite()) {
        return Err(GeneralizedTacticValueError::FeatureWidth);
    }
    let action = encode_action(context, descriptor)?;
    let mut features = Vec::with_capacity(
        state_features.len()
            + GENERALIZED_TACTIC_BEHAVIOR_CONTEXT_WIDTH
            + GENERALIZED_TACTIC_ACTION_FEATURE_WIDTH,
    );
    features.extend_from_slice(state_features);
    features.extend_from_slice(&context.values());
    features.extend_from_slice(&action);
    Ok(features)
}

pub(crate) fn action_class(option_type: &OptionType) -> u32 {
    option_type_index(option_type) as u32
}

fn state_neighbors<'a>(
    model: &'a GeneralizedTacticValueModel,
    state_features: &[f32],
) -> Result<Vec<(f32, &'a EncodedSample)>, GeneralizedTacticValueError> {
    // Reaching a hindsight coordinate and satisfying the authored game
    // predicate are different tasks, even at the same position.
    let query;
    let state_features = if let Some(kind) = &model.goal_query_kind {
        query = state_features
            .iter()
            .copied()
            .chain(std::iter::once(kind.feature()))
            .collect::<Vec<_>>();
        query.as_slice()
    } else {
        state_features
    };
    if state_features.len() != model.state_min.len()
        || state_features.iter().any(|value| !value.is_finite())
    {
        return Err(GeneralizedTacticValueError::FeatureWidth);
    }

    // State distance is independent of the candidate action. Computing and
    // sorting it once is critical when one decision ranks a full parameterized
    // controller lattice against thousands of replay rows.
    let mut state_neighbors = model
        .samples
        .iter()
        .map(|sample| {
            (
                weighted_normalized_distance(
                    state_features,
                    &sample.state,
                    &model.state_min,
                    &model.state_range,
                    &model.state_distance_weights,
                ),
                sample,
            )
        })
        .collect::<Vec<_>>();
    state_neighbors.sort_by(|left, right| left.0.total_cmp(&right.0));
    if state_neighbors
        .first()
        .is_some_and(|(distance, _)| *distance <= EXACT_STATE_DISTANCE_EPSILON)
    {
        state_neighbors.retain(|(distance, _)| *distance <= EXACT_STATE_DISTANCE_EPSILON);
    }
    Ok(state_neighbors)
}

pub(super) fn estimate_actions(
    model: &GeneralizedTacticValueModel,
    state_features: &[f32],
    context: &GeneralizedTacticContext,
    descriptors: &[OptionActionDescriptor],
) -> Result<Vec<GeneralizedTacticEstimate>, GeneralizedTacticValueError> {
    let state_neighbors = state_neighbors(model, state_features)?;

    // The nearest terminal state cohort is likewise independent of the action.
    // Absolute simulation/tape position is deliberately absent: successful
    // demonstration timing is not a privileged route-phase key.
    let behavior_context = context.values();
    let terminal_distances = model
        .samples
        .iter()
        .filter(|sample| sample.outcome.terminal > 0.0)
        .map(|sample| {
            (
                normalized_distance(
                    &behavior_context,
                    &sample.behavior_context,
                    &model.behavior_context_min,
                    &model.behavior_context_range,
                ),
                sample,
            )
        })
        .collect::<Vec<_>>();
    let minimum_state_distance = terminal_distances
        .iter()
        .map(|(state_distance, _)| *state_distance)
        .min_by(f32::total_cmp);
    let terminal_cohort = minimum_state_distance.map_or_else(Vec::new, |minimum_state_distance| {
        terminal_distances
            .iter()
            .filter(|(state_distance, _)| {
                *state_distance <= minimum_state_distance + EXACT_STATE_DISTANCE_EPSILON
            })
            .map(|(_, sample)| *sample)
            .collect::<Vec<_>>()
    });

    descriptors
        .iter()
        .map(|descriptor| {
            estimate_action(
                model,
                context,
                descriptor,
                &state_neighbors,
                &terminal_cohort,
            )
        })
        .collect()
}

fn action_neighbors<'a>(
    model: &GeneralizedTacticValueModel,
    action: &[f32; GENERALIZED_TACTIC_ACTION_FEATURE_WIDTH],
    state_neighbors: &[(f32, &'a EncodedSample)],
) -> Vec<(f32, &'a EncodedSample)> {
    // State-only truncation can discard the nearest state-action examples:
    // many relabeled goals for one action crowd out a different action before
    // its distance is even evaluated. Keep the exact nearest joint neighbors.
    // Sorted state distance is a lower bound because action distance >= 0;
    // stop only when no remaining row can enter the bounded best-neighbor set.
    let mut neighbors: Vec<(f32, &'a EncodedSample)> = Vec::with_capacity(NEIGHBORS + 1);
    for (state_distance, sample) in state_neighbors {
        if neighbors.len() == NEIGHBORS && *state_distance > neighbors[NEIGHBORS - 1].0 {
            break;
        }
        let distance = *state_distance
            + normalized_distance(
                action,
                &sample.action,
                &model.action_min,
                &model.action_range,
            ) * 2.0;
        // Insert after equal distances to preserve stable state-cohort order.
        let position = neighbors.partition_point(|(existing, _)| *existing <= distance);
        if position < NEIGHBORS {
            neighbors.insert(position, (distance, *sample));
            neighbors.truncate(NEIGHBORS);
        }
    }
    neighbors
}

pub(super) fn explain_action(
    model: &GeneralizedTacticValueModel,
    state_features: &[f32],
    context: &GeneralizedTacticContext,
    descriptor: &OptionActionDescriptor,
) -> Result<Vec<GeneralizedTacticNeighbor>, GeneralizedTacticValueError> {
    let states = state_neighbors(model, state_features)?;
    let action = encode_action(context, descriptor)?;
    let neighbors = action_neighbors(model, &action, &states);
    let total_weight: f32 = neighbors.iter().map(|(d, _)| 1.0 / (0.01 + d)).sum();
    Ok(neighbors
        .iter()
        .map(|(distance, sample)| {
            let state_distance = states
                .iter()
                .find(|(_, state)| std::ptr::eq(*state, *sample))
                .expect("action neighbor belongs to state cohort")
                .0;
            GeneralizedTacticNeighbor {
                training_sample_index: model
                    .samples
                    .iter()
                    .position(|candidate| std::ptr::eq(candidate, *sample))
                    .expect("neighbor belongs to model"),
                state_distance,
                action_distance: (distance - state_distance) / 2.0,
                normalized_weight: (1.0 / (0.01 + distance)) / total_weight,
                state_features: sample.state.clone(),
                outcome: sample.outcome,
            }
        })
        .collect())
}

fn estimate_action(
    model: &GeneralizedTacticValueModel,
    context: &GeneralizedTacticContext,
    descriptor: &OptionActionDescriptor,
    state_neighbors: &[(f32, &EncodedSample)],
    terminal_cohort: &[&EncodedSample],
) -> Result<GeneralizedTacticEstimate, GeneralizedTacticValueError> {
    let action = encode_action(context, descriptor)?;
    let neighbors = action_neighbors(model, &action, state_neighbors);
    let nearest_distance = neighbors[0].0;
    let terminal_support_distance = (!terminal_cohort.is_empty()).then(|| {
        terminal_cohort
            .iter()
            .map(|sample| {
                behavior_cloning_action_distance(
                    &action,
                    &sample.action,
                    &model.action_min,
                    &model.action_range,
                )
            })
            .min_by(f32::total_cmp)
            .expect("nonempty nearest terminal-state cohort")
    });
    let mut outcome = GeneralizedTacticOutcome::default();
    let mut total_weight = 0.0_f32;
    let mut terminal_weight = 0.0_f32;
    let mut terminal_duration = 0.0_f32;
    for (distance, sample) in &neighbors {
        let weight = 1.0 / (0.01 + *distance);
        outcome.weighted_add(sample.outcome, weight);
        total_weight += weight;
        let supported_weight = weight * sample.outcome.terminal.clamp(0.0, 1.0);
        terminal_weight += supported_weight;
        terminal_duration += sample.outcome.duration_ticks * supported_weight;
    }
    outcome.scale(1.0 / total_weight);
    outcome.duration_ticks = if terminal_weight > 0.0 {
        terminal_duration / terminal_weight
    } else {
        0.0
    };
    Ok(GeneralizedTacticEstimate {
        descriptor: descriptor.clone(),
        outcome,
        nearest_distance,
        terminal_support_distance,
        neighbors: neighbors.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn bounded_joint_neighbors_match_exhaustive_search() {
        let actions = (0..5)
            .map(|index| OptionActionDescriptor {
                option_id: format!("action-{index}"),
                option_type: OptionType::Move,
                parameters: BTreeMap::from([
                    (
                        "duration_ticks".into(),
                        OptionParameter::Unsigned(index + 1),
                    ),
                    (
                        "command_initial_heading".into(),
                        OptionParameter::F32Bits((index as f32).to_bits()),
                    ),
                ]),
            })
            .collect::<Vec<_>>();
        let samples = (0..80)
            .flat_map(|index| {
                actions
                    .iter()
                    .map(move |action| GeneralizedTacticTrainingSample {
                        state_features: vec![(index % 20) as f32, (index % 3) as f32],
                        context: GeneralizedTacticContext::default(),
                        action: action.clone(),
                        outcome: GeneralizedTacticOutcome::default(),
                    })
            })
            .collect::<Vec<_>>();
        let model = GeneralizedTacticValueModel::fit(&samples).unwrap();
        for query in [[0.0, 0.0], [0.01, 0.01], [12.5, 1.5], [1000.0, -20.0]] {
            let states = state_neighbors(&model, &query).unwrap();
            for descriptor in &actions {
                let action =
                    encode_action(&GeneralizedTacticContext::default(), descriptor).unwrap();
                let actual = action_neighbors(&model, &action, &states);
                let mut expected = states
                    .iter()
                    .map(|(distance, row)| {
                        (
                            distance
                                + 2.0
                                    * normalized_distance(
                                        &action,
                                        &row.action,
                                        &model.action_min,
                                        &model.action_range,
                                    ),
                            *row,
                        )
                    })
                    .collect::<Vec<_>>();
                expected.sort_by(|left, right| left.0.total_cmp(&right.0));
                expected.truncate(NEIGHBORS);
                assert_eq!(actual.len(), expected.len());
                for (actual, expected) in actual.iter().zip(&expected) {
                    assert_eq!(actual.0, expected.0);
                    assert!(std::ptr::eq(actual.1, expected.1));
                }
            }
        }
    }
}
