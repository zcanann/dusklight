//! Fit and score held-out native goal-reachability models.

use super::*;

#[derive(Clone, Copy)]
pub(super) struct TrainingBaseline {
    success_probability: f64,
    successful_time_ticks: f64,
    discounted_return: f64,
    discounted_tick_cost: f64,
}

impl TrainingBaseline {
    pub(super) fn from_rows(
        rows: &[MaterializedRow],
        indices: &[usize],
    ) -> Result<Self, NativeGoalReachabilityError> {
        let success_probability = indices.iter().filter(|index| rows[**index].success).count()
            as f64
            / indices.len() as f64;
        let successful_times = indices
            .iter()
            .filter_map(|index| rows[*index].ticks_to_goal.map(f64::from))
            .collect::<Vec<_>>();
        let discounted_return = indices
            .iter()
            .map(|index| realized_return(&rows[*index]))
            .sum::<f64>()
            / indices.len() as f64;
        let by_identity = rows
            .iter()
            .map(|row| (row.row_sha256, row))
            .collect::<BTreeMap<_, _>>();
        let discounted_tick_cost = indices
            .iter()
            .map(|index| realized_cost(&by_identity, &rows[*index]))
            .collect::<Result<Vec<_>, _>>()?
            .iter()
            .sum::<f64>()
            / indices.len() as f64;
        if successful_times.is_empty() {
            return Err(NativeGoalReachabilityError::new(
                "training baseline has no successful time targets",
            ));
        }
        Ok(Self {
            success_probability,
            successful_time_ticks: successful_times.iter().sum::<f64>()
                / successful_times.len() as f64,
            discounted_return,
            discounted_tick_cost,
        })
    }
}

pub(super) fn evaluate(
    rows: &[MaterializedRow],
    normalized: &[Vec<f64>],
    members: &[ReachabilityMember],
    split: AuxiliarySplit,
    baseline: TrainingBaseline,
    hidden_width: usize,
) -> Result<NativeGoalReachabilityMetrics, NativeGoalReachabilityError> {
    let indices = split_indices(rows, split);
    let mut reachability_squared = 0.0;
    let mut baseline_reachability_squared = 0.0;
    let mut time_absolute = 0.0;
    let mut baseline_time_absolute = 0.0;
    let mut time_support = 0_usize;
    let mut return_squared = 0.0;
    let mut baseline_return_squared = 0.0;
    let mut cost_absolute = 0.0;
    let mut baseline_cost_absolute = 0.0;
    let mut reachability_stddev = 0.0;
    let mut return_stddev = 0.0;
    let mut successful_rows = 0;
    let by_identity = rows
        .iter()
        .map(|row| (row.row_sha256, row))
        .collect::<BTreeMap<_, _>>();
    for index in &indices {
        let row = &rows[*index];
        let estimate = ensemble_estimate(members, &normalized[*index], hidden_width)?;
        let expected_success = f64::from(row.success);
        reachability_squared += (estimate.reachability_probability - expected_success).powi(2);
        baseline_reachability_squared += (baseline.success_probability - expected_success).powi(2);
        if let Some(ticks) = row.ticks_to_goal {
            time_absolute += (estimate.expected_ticks_to_goal - f64::from(ticks)).abs();
            baseline_time_absolute += (baseline.successful_time_ticks - f64::from(ticks)).abs();
            time_support += 1;
            successful_rows += 1;
        }
        let expected_return = realized_return(row);
        return_squared += (estimate.discounted_terminal_return - expected_return).powi(2);
        baseline_return_squared += (baseline.discounted_return - expected_return).powi(2);
        let expected_cost = realized_cost(&by_identity, row)?;
        cost_absolute += (estimate.discounted_tick_cost - expected_cost).abs();
        baseline_cost_absolute += (baseline.discounted_tick_cost - expected_cost).abs();
        reachability_stddev += estimate.reachability_stddev;
        return_stddev += estimate.return_stddev;
    }
    let count = indices.len() as f64;
    let reachability_brier = reachability_squared / count;
    let baseline_reachability_brier = baseline_reachability_squared / count;
    let discounted_return_rmse = (return_squared / count).sqrt();
    let baseline_discounted_return_rmse = (baseline_return_squared / count).sqrt();
    let metrics = NativeGoalReachabilityMetrics {
        rows: indices.len(),
        episodes: indices
            .iter()
            .map(|index| rows[*index].episode_sha256)
            .collect::<BTreeSet<_>>()
            .len(),
        successful_rows,
        failed_rows: indices.len() - successful_rows,
        reachability_brier,
        baseline_reachability_brier,
        reachability_relative_improvement: relative_improvement(
            baseline_reachability_brier,
            reachability_brier,
        ),
        successful_time_mae_ticks: time_absolute / time_support as f64,
        baseline_successful_time_mae_ticks: baseline_time_absolute / time_support as f64,
        successful_time_relative_improvement: relative_improvement(
            baseline_time_absolute / time_support as f64,
            time_absolute / time_support as f64,
        ),
        discounted_return_rmse,
        baseline_discounted_return_rmse,
        return_relative_improvement: relative_improvement(
            baseline_discounted_return_rmse,
            discounted_return_rmse,
        ),
        discounted_tick_cost_mae: cost_absolute / count,
        baseline_discounted_tick_cost_mae: baseline_cost_absolute / count,
        tick_cost_relative_improvement: relative_improvement(
            baseline_cost_absolute / count,
            cost_absolute / count,
        ),
        mean_reachability_stddev: reachability_stddev / count,
        mean_return_stddev: return_stddev / count,
    };
    if !metrics.validate() {
        return Err(NativeGoalReachabilityError::new(
            "goal reachability metrics are invalid",
        ));
    }
    Ok(metrics)
}

pub(super) fn admission(
    validation: NativeGoalReachabilityMetrics,
    config: NativeGoalReachabilityConfig,
) -> NativeGoalReachabilityAdmission {
    if validation.reachability_relative_improvement >= config.minimum_validation_improvement
        && validation.return_relative_improvement >= config.minimum_validation_improvement
        && validation.successful_time_relative_improvement >= config.minimum_validation_improvement
        && validation.tick_cost_relative_improvement >= config.minimum_validation_improvement
        && validation.mean_reachability_stddev <= config.maximum_validation_reachability_stddev
    {
        NativeGoalReachabilityAdmission::GoalConditionedCandidate
    } else {
        NativeGoalReachabilityAdmission::RetainTrainingMeanBaseline
    }
}

pub(super) fn ensemble_estimate(
    members: &[ReachabilityMember],
    features: &[f64],
    hidden_width: usize,
) -> Result<NativeGoalReachabilityEstimate, NativeGoalReachabilityError> {
    let predictions = members
        .iter()
        .map(|member| member.forward(features, hidden_width).prediction)
        .collect::<Vec<_>>();
    if predictions.is_empty() {
        return Err(NativeGoalReachabilityError::new(
            "reachability ensemble has no members",
        ));
    }
    let mean = std::array::from_fn::<_, NATIVE_GOAL_REACHABILITY_HEADS, _>(|head| {
        predictions
            .iter()
            .map(|prediction| prediction.values[head])
            .sum::<f64>()
            / predictions.len() as f64
    });
    let stddev = |head: usize| {
        (predictions
            .iter()
            .map(|prediction| (prediction.values[head] - mean[head]).powi(2))
            .sum::<f64>()
            / predictions.len() as f64)
            .sqrt()
    };
    let estimate = NativeGoalReachabilityEstimate {
        reachability_probability: mean[HEAD_SUCCESS],
        reachability_stddev: stddev(HEAD_SUCCESS),
        expected_ticks_to_goal: denormalize_ticks(mean[HEAD_TIME]),
        discounted_terminal_return: mean[HEAD_RETURN],
        return_stddev: stddev(HEAD_RETURN),
        discounted_tick_cost: denormalize_ticks(mean[HEAD_COST]),
    };
    if [
        estimate.reachability_probability,
        estimate.reachability_stddev,
        estimate.expected_ticks_to_goal,
        estimate.discounted_terminal_return,
        estimate.return_stddev,
        estimate.discounted_tick_cost,
    ]
    .iter()
    .any(|value| !value.is_finite() || *value < 0.0)
    {
        return Err(NativeGoalReachabilityError::new(
            "reachability ensemble estimate is invalid",
        ));
    }
    Ok(estimate)
}

pub(super) fn realized_return(row: &MaterializedRow) -> f64 {
    row.realized_return
}

pub(super) fn realized_cost(
    by_identity: &BTreeMap<Digest, &MaterializedRow>,
    row: &MaterializedRow,
) -> Result<f64, NativeGoalReachabilityError> {
    let mut total = row.discounted_tick_cost;
    let mut discount = row.bootstrap_discount;
    let mut next = row.bootstrap_row_sha256;
    let mut visited = BTreeSet::new();
    while let Some(identity) = next {
        if !visited.insert(identity) {
            return Err(NativeGoalReachabilityError::new(
                "discounted cost bootstrap contains a cycle",
            ));
        }
        let target = by_identity.get(&identity).ok_or_else(|| {
            NativeGoalReachabilityError::new("discounted cost bootstrap row is absent")
        })?;
        total += discount * target.discounted_tick_cost;
        discount *= target.bootstrap_discount;
        next = target.bootstrap_row_sha256;
    }
    Ok(total)
}

pub(super) fn relative_improvement(baseline: f64, model: f64) -> f64 {
    if baseline > f64::EPSILON {
        ((baseline - model) / baseline).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

pub(super) fn fit_normalization(
    rows: &[MaterializedRow],
    training: &[usize],
    width: usize,
) -> Result<(Vec<f64>, Vec<f64>), NativeGoalReachabilityError> {
    let mut mean = vec![0.0; width];
    for index in training {
        for (output, feature) in mean.iter_mut().zip(&rows[*index].features) {
            *output += feature;
        }
    }
    for value in &mut mean {
        *value /= training.len() as f64;
    }
    let mut variance = vec![0.0; width];
    for index in training {
        for ((output, feature), mean) in variance.iter_mut().zip(&rows[*index].features).zip(&mean)
        {
            *output += (feature - mean).powi(2);
        }
    }
    let inverse_stddev = variance
        .into_iter()
        .map(|value| {
            let stddev = (value / training.len() as f64).sqrt();
            if stddev > 1.0e-8 { 1.0 / stddev } else { 1.0 }
        })
        .collect::<Vec<_>>();
    if mean
        .iter()
        .chain(&inverse_stddev)
        .any(|value| !value.is_finite())
    {
        return Err(NativeGoalReachabilityError::new(
            "training-only normalization became non-finite",
        ));
    }
    Ok((mean, inverse_stddev))
}

pub(super) fn normalize(features: &[f64], mean: &[f64], inverse_stddev: &[f64]) -> Vec<f64> {
    features
        .iter()
        .zip(mean)
        .zip(inverse_stddev)
        .map(|((value, mean), inverse)| (value - mean) * inverse)
        .collect()
}

pub(super) fn split_indices(rows: &[MaterializedRow], split: AuxiliarySplit) -> Vec<usize> {
    rows.iter()
        .enumerate()
        .filter_map(|(index, row)| (row.split == split).then_some(index))
        .collect()
}

pub(super) fn group_training_episodes(
    rows: &[MaterializedRow],
    training: &[usize],
) -> BTreeMap<Digest, Vec<usize>> {
    let mut groups = BTreeMap::new();
    for index in training {
        groups
            .entry(rows[*index].episode_sha256)
            .or_insert_with(Vec::new)
            .push(*index);
    }
    groups
}

pub(super) fn episode_identity(row: &NativeGoalTrajectoryRow) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight.physical-native-episode/v1\0");
    hasher.update(row.shard_sha256.0);
    hasher.update(row.episode_payload_xxh3_128.as_bytes());
    Digest(hasher.finalize().into())
}
