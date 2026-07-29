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
    let backup_limit = fitted_q_backup_limit(minimum_iterations, transitions.len());
    let exact_terminal_supported = terminal_supported_transition_indices(transitions);
    let exact_first_hit_ticks =
        terminal_supported_first_hit_ticks(transitions, &exact_terminal_supported, backup_limit)?;
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
