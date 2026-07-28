use super::*;

pub(super) fn estimate_actions(
    model: &GeneralizedTacticValueModel,
    state_features: &[f32],
    context: &GeneralizedTacticContext,
    descriptors: &[OptionActionDescriptor],
) -> Result<Vec<GeneralizedTacticEstimate>, GeneralizedTacticValueError> {
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
                normalized_distance(
                    state_features,
                    &sample.state,
                    &model.state_min,
                    &model.state_range,
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
    } else {
        state_neighbors.truncate(STATE_NEIGHBORS.min(state_neighbors.len()));
    }

    // The nearest terminal phase/state cohort is likewise independent of the
    // action. Only the final behavior-cloning action distance varies.
    let behavior_context = context.values();
    let terminal_distances = model
        .samples
        .iter()
        .filter(|sample| sample.outcome.terminal > 0.0)
        .map(|sample| {
            (
                (behavior_context[0] - sample.behavior_context[0]).abs(),
                normalized_distance(
                    &behavior_context[1..],
                    &sample.behavior_context[1..],
                    &model.behavior_context_min[1..],
                    &model.behavior_context_range[1..],
                ),
                sample,
            )
        })
        .collect::<Vec<_>>();
    let minimum_tick_distance = terminal_distances
        .iter()
        .map(|(tick_distance, _, _)| *tick_distance)
        .min_by(f32::total_cmp);
    let minimum_state_distance = minimum_tick_distance.and_then(|minimum_tick_distance| {
        terminal_distances
            .iter()
            .filter(|(tick_distance, _, _)| {
                *tick_distance <= minimum_tick_distance + EXACT_STATE_DISTANCE_EPSILON
            })
            .map(|(_, state_distance, _)| *state_distance)
            .min_by(f32::total_cmp)
    });
    let terminal_cohort = minimum_tick_distance
        .zip(minimum_state_distance)
        .map_or_else(
            Vec::new,
            |(minimum_tick_distance, minimum_state_distance)| {
                terminal_distances
                    .iter()
                    .filter(|(tick_distance, state_distance, _)| {
                        *tick_distance <= minimum_tick_distance + EXACT_STATE_DISTANCE_EPSILON
                            && *state_distance
                                <= minimum_state_distance + EXACT_STATE_DISTANCE_EPSILON
                    })
                    .map(|(_, _, sample)| *sample)
                    .collect::<Vec<_>>()
            },
        );

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

fn estimate_action(
    model: &GeneralizedTacticValueModel,
    context: &GeneralizedTacticContext,
    descriptor: &OptionActionDescriptor,
    state_neighbors: &[(f32, &EncodedSample)],
    terminal_cohort: &[&EncodedSample],
) -> Result<GeneralizedTacticEstimate, GeneralizedTacticValueError> {
    let action = encode_action(context, descriptor)?;
    let mut neighbors = state_neighbors
        .iter()
        .map(|(state_distance, sample)| {
            (
                *state_distance
                    + normalized_distance(
                        &action,
                        &sample.action,
                        &model.action_min,
                        &model.action_range,
                    ) * 2.0,
                *sample,
            )
        })
        .collect::<Vec<_>>();
    neighbors.sort_by(|left, right| left.0.total_cmp(&right.0));
    neighbors.truncate(NEIGHBORS.min(neighbors.len()));
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
