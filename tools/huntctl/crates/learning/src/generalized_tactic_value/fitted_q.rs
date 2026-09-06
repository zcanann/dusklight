use super::*;

pub(crate) struct FittedQResult {
    /// Negative conditional ticks-to-terminal for replay rows with an
    /// authenticated terminal continuation. Open episode ends remain `None`;
    /// they are censored observations, not zero-return failures.
    pub values: Vec<Option<f32>>,
    pub exact_terminal_supported: BTreeSet<usize>,
    pub exact_first_hit_ticks: Vec<Option<u64>>,
}

pub(crate) fn fit_transition_returns(
    transitions: &[OptionTransitionSample],
    minimum_iterations: usize,
    per_tick_discount: f32,
) -> Result<FittedQResult, GeneralizedTacticValueError> {
    if minimum_iterations == 0
        || minimum_iterations > MAX_FITTED_Q_BACKUP_ITERATIONS
        || !per_tick_discount.is_finite()
        || !(0.0..=1.0).contains(&per_tick_discount)
        || per_tick_discount == 0.0
    {
        return Err(GeneralizedTacticValueError::InvalidConfig);
    }
    for transition in transitions {
        transition
            .validate()
            .map_err(|error| GeneralizedTacticValueError::InvalidTransition(error.to_string()))?;
    }
    // These targets are exact recorded path costs, not iterative function
    // approximation. Training settings remain validated above for existing
    // callers, but may not truncate an authenticated continuation.
    let edges = transitions
        .iter()
        .map(|transition| {
            (
                transition.before_state_sha256,
                transition.after_state_sha256,
                transition.value_sample.duration_ticks,
                transition.value_sample.terminal,
            )
        })
        .collect::<Vec<_>>();
    let exact_first_hit_ticks = reverse_costs::terminal_edge_costs(&edges);
    let exact_terminal_supported = exact_first_hit_ticks
        .iter()
        .enumerate()
        .filter_map(|(index, ticks)| ticks.is_some().then_some(index))
        .collect();
    let values = exact_first_hit_ticks
        .iter()
        .map(|ticks| ticks.map(|ticks| -(ticks as f32)))
        .collect();
    Ok(FittedQResult {
        values,
        exact_terminal_supported,
        exact_first_hit_ticks,
    })
}
