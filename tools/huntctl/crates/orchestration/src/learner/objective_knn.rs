use super::{GraphLearnerError, LearnedGraphActionEstimate};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ActionObjectiveSample {
    source_feature_f32_bits: Vec<u32>,
    conditional_ticks_to_terminal: u64,
}

impl ActionObjectiveSample {
    pub(super) fn new(source_features: &[f32], conditional_ticks_to_terminal: u64) -> Self {
        Self {
            source_feature_f32_bits: source_features
                .iter()
                .map(|value| value.to_bits())
                .collect(),
            conditional_ticks_to_terminal,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ActionObjectiveModel {
    samples: Vec<ActionObjectiveSample>,
    feature_min_f32_bits: Vec<u32>,
    feature_max_f32_bits: Vec<u32>,
    mean_conditional_ticks_to_terminal: u64,
    leave_one_out_error_millionths: u64,
}

impl ActionObjectiveModel {
    pub(super) fn fit(samples: Vec<ActionObjectiveSample>) -> Result<Self, GraphLearnerError> {
        let width = samples
            .first()
            .map(|sample| sample.source_feature_f32_bits.len())
            .ok_or(GraphLearnerError::Invalid(
                "objective model requires terminal-connected samples",
            ))?;
        if width == 0
            || samples
                .iter()
                .any(|sample| sample.source_feature_f32_bits.len() != width)
        {
            return Err(GraphLearnerError::Invalid(
                "objective model has incompatible feature widths",
            ));
        }
        let mut minimum = vec![f32::INFINITY; width];
        let mut maximum = vec![f32::NEG_INFINITY; width];
        let mut tick_sum = 0_u128;
        for sample in &samples {
            tick_sum = tick_sum.saturating_add(u128::from(sample.conditional_ticks_to_terminal));
            for ((minimum, maximum), value) in minimum
                .iter_mut()
                .zip(&mut maximum)
                .zip(&sample.source_feature_f32_bits)
            {
                let value = f32::from_bits(*value);
                *minimum = minimum.min(value);
                *maximum = maximum.max(value);
            }
        }
        let mean_conditional_ticks_to_terminal =
            u64::try_from(tick_sum / samples.len().max(1) as u128).unwrap_or(u64::MAX);
        let mut model = Self {
            samples,
            feature_min_f32_bits: minimum.into_iter().map(f32::to_bits).collect(),
            feature_max_f32_bits: maximum.into_iter().map(f32::to_bits).collect(),
            mean_conditional_ticks_to_terminal,
            leave_one_out_error_millionths: 1_000_000,
        };
        model.leave_one_out_error_millionths = model.leave_one_out_error();
        Ok(model)
    }

    pub(super) fn predict(&self, source_features: &[f32]) -> Option<LearnedGraphActionEstimate> {
        let prediction = self.predict_excluding(source_features, None)?;
        Some(LearnedGraphActionEstimate {
            terminal_support_per_million: None,
            conditional_ticks_to_terminal: Some(prediction.ticks),
            uncertainty_millionths: prediction.uncertainty_millionths,
            prediction_error_millionths: self.leave_one_out_error_millionths,
        })
    }

    pub(super) fn mean_prediction(&self) -> LearnedGraphActionEstimate {
        LearnedGraphActionEstimate {
            terminal_support_per_million: None,
            conditional_ticks_to_terminal: Some(self.mean_conditional_ticks_to_terminal),
            uncertainty_millionths: 1_000_000,
            prediction_error_millionths: self.leave_one_out_error_millionths,
        }
    }

    fn leave_one_out_error(&self) -> u64 {
        if self.samples.len() <= 1 {
            return 1_000_000;
        }
        let mut error = 0_u128;
        let mut predictions = 0_u128;
        for (index, sample) in self.samples.iter().enumerate() {
            let features = sample
                .source_feature_f32_bits
                .iter()
                .map(|value| f32::from_bits(*value))
                .collect::<Vec<_>>();
            if let Some(prediction) = self.predict_excluding(&features, Some(index)) {
                error = error.saturating_add(u128::from(relative_tick_error(
                    sample.conditional_ticks_to_terminal,
                    prediction.ticks,
                )));
                predictions += 1;
            }
        }
        u64::try_from(error / predictions.max(1)).unwrap_or(u64::MAX)
    }

    fn predict_excluding(
        &self,
        source_features: &[f32],
        excluded_index: Option<usize>,
    ) -> Option<ObjectivePrediction> {
        if source_features.len() != self.feature_min_f32_bits.len()
            || source_features.iter().any(|value| !value.is_finite())
        {
            return None;
        }
        let mut neighbors = self
            .samples
            .iter()
            .enumerate()
            .filter(|(index, _)| Some(*index) != excluded_index)
            .map(|(index, sample)| {
                (
                    self.normalized_feature_distance(source_features, sample),
                    index,
                    sample.conditional_ticks_to_terminal,
                )
            })
            .collect::<Vec<_>>();
        if neighbors.is_empty() {
            return None;
        }
        neighbors.sort_by(|left, right| {
            left.0
                .total_cmp(&right.0)
                .then_with(|| left.1.cmp(&right.1))
        });
        neighbors.truncate(3);
        let ticks = if let Some(exact) = neighbors.iter().find(|neighbor| neighbor.0 == 0.0) {
            exact.2
        } else {
            let mut weighted_ticks = 0.0_f64;
            let mut weight_sum = 0.0_f64;
            for (distance, _, ticks) in &neighbors {
                let weight = 1.0 / (f64::from(*distance) + 1.0e-6);
                weighted_ticks += weight * *ticks as f64;
                weight_sum += weight;
            }
            (weighted_ticks / weight_sum)
                .round()
                .clamp(0.0, u64::MAX as f64) as u64
        };
        let nearest_distance = neighbors[0].0.clamp(0.0, 1.0);
        let dispersion = neighbors
            .iter()
            .map(|neighbor| relative_tick_error(ticks, neighbor.2))
            .sum::<u64>()
            / neighbors.len().max(1) as u64;
        Some(ObjectivePrediction {
            ticks,
            uncertainty_millionths: ((f64::from(nearest_distance) * 1_000_000.0).round() as u64)
                .saturating_add(dispersion)
                .min(1_000_000),
        })
    }

    fn normalized_feature_distance(
        &self,
        source_features: &[f32],
        sample: &ActionObjectiveSample,
    ) -> f32 {
        let squared = source_features
            .iter()
            .zip(&sample.source_feature_f32_bits)
            .zip(
                self.feature_min_f32_bits
                    .iter()
                    .zip(&self.feature_max_f32_bits),
            )
            .map(|((query, sample), (minimum, maximum))| {
                let sample = f32::from_bits(*sample);
                let minimum = f32::from_bits(*minimum);
                let maximum = f32::from_bits(*maximum);
                let range = maximum - minimum;
                let scale = if range.abs() <= f32::EPSILON {
                    minimum.abs().max(maximum.abs()).max(1.0)
                } else {
                    range
                };
                let delta = (*query - sample) / scale;
                delta * delta
            })
            .sum::<f32>();
        (squared / source_features.len().max(1) as f32).sqrt()
    }
}

#[derive(Clone, Copy, Debug)]
struct ObjectivePrediction {
    ticks: u64,
    uncertainty_millionths: u64,
}

fn relative_tick_error(actual: u64, predicted: u64) -> u64 {
    u64::try_from(
        u128::from(actual.abs_diff(predicted)).saturating_mul(1_000_000)
            / u128::from(actual.max(predicted).max(1)),
    )
    .unwrap_or(u64::MAX)
}
