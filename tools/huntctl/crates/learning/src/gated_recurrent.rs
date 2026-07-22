//! Small deterministic GRU with explicit bounded backpropagation through time.
//!
//! This is an internal representation primitive. It has no environment or
//! controller authority: callers supply already authenticated, past-only
//! observations and retain responsibility for phase correctness.

use crate::trainable_set_encoder::{
    DeterministicRng, TrainableSetError, clip, initialized_weights,
};
use serde::Serialize;

const GATE_COUNT: usize = 3;
const RESET_GATE: usize = 0;
const UPDATE_GATE: usize = 1;
const CANDIDATE_GATE: usize = 2;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct GatedRecurrent {
    input_width: usize,
    hidden_width: usize,
    input_weights: Vec<f64>,
    recurrent_weights: Vec<f64>,
    bias: Vec<f64>,
}

#[derive(Clone, Debug)]
pub(crate) struct GatedRecurrentStep {
    pub(crate) input: Vec<f64>,
    pub(crate) previous_hidden: Vec<f64>,
    pub(crate) reset: Vec<f64>,
    pub(crate) update: Vec<f64>,
    pub(crate) candidate: Vec<f64>,
    pub(crate) hidden: Vec<f64>,
}

#[derive(Clone, Debug)]
pub(crate) struct GatedRecurrentGradients {
    pub(crate) input_weights: Vec<f64>,
    pub(crate) recurrent_weights: Vec<f64>,
    pub(crate) bias: Vec<f64>,
}

impl GatedRecurrent {
    pub(crate) fn initialized(
        input_width: usize,
        hidden_width: usize,
        rng: &mut DeterministicRng,
    ) -> Result<Self, TrainableSetError> {
        if input_width == 0 || hidden_width == 0 {
            return Err(TrainableSetError::new(
                "gated recurrent widths must be nonzero",
            ));
        }
        let gated_hidden_width = hidden_width
            .checked_mul(GATE_COUNT)
            .ok_or_else(|| TrainableSetError::new("gated recurrent width overflowed"))?;
        Ok(Self {
            input_width,
            hidden_width,
            input_weights: initialized_weights(gated_hidden_width, input_width, rng),
            recurrent_weights: initialized_weights(gated_hidden_width, hidden_width, rng),
            bias: vec![0.0; gated_hidden_width],
        })
    }

    pub(crate) fn input_width(&self) -> usize {
        self.input_width
    }

    pub(crate) fn hidden_width(&self) -> usize {
        self.hidden_width
    }

    pub(crate) fn parameter_count(&self) -> usize {
        self.input_weights.len() + self.recurrent_weights.len() + self.bias.len()
    }

    pub(crate) fn all_finite(&self) -> bool {
        self.input_weights
            .iter()
            .chain(&self.recurrent_weights)
            .chain(&self.bias)
            .all(|value| value.is_finite())
    }

    pub(crate) fn forward_sequence(
        &self,
        inputs: &[Vec<f64>],
    ) -> Result<Vec<GatedRecurrentStep>, TrainableSetError> {
        let mut hidden = vec![0.0; self.hidden_width];
        let mut steps = Vec::with_capacity(inputs.len());
        for input in inputs {
            if input.len() != self.input_width || input.iter().any(|value| !value.is_finite()) {
                return Err(TrainableSetError::new(
                    "gated recurrent input width or value is invalid",
                ));
            }
            let step = self.forward_step(input, &hidden);
            hidden.clone_from(&step.hidden);
            steps.push(step);
        }
        Ok(steps)
    }

    fn forward_step(&self, input: &[f64], previous_hidden: &[f64]) -> GatedRecurrentStep {
        let reset = (0..self.hidden_width)
            .map(|unit| logistic(self.gate_affine(RESET_GATE, unit, input, previous_hidden)))
            .collect::<Vec<_>>();
        let update = (0..self.hidden_width)
            .map(|unit| logistic(self.gate_affine(UPDATE_GATE, unit, input, previous_hidden)))
            .collect::<Vec<_>>();
        let reset_hidden = reset
            .iter()
            .zip(previous_hidden)
            .map(|(gate, hidden)| gate * hidden)
            .collect::<Vec<_>>();
        let candidate = (0..self.hidden_width)
            .map(|unit| {
                self.gate_affine(CANDIDATE_GATE, unit, input, &reset_hidden)
                    .tanh()
            })
            .collect::<Vec<_>>();
        let hidden = previous_hidden
            .iter()
            .zip(&update)
            .zip(&candidate)
            .map(|((previous, update), candidate)| (1.0 - update) * previous + update * candidate)
            .collect();
        GatedRecurrentStep {
            input: input.to_vec(),
            previous_hidden: previous_hidden.to_vec(),
            reset,
            update,
            candidate,
            hidden,
        }
    }

    fn gate_affine(&self, gate: usize, unit: usize, input: &[f64], recurrent_input: &[f64]) -> f64 {
        let input_offset = gate_row_offset(gate, unit, self.hidden_width, self.input_width);
        let recurrent_offset = gate_row_offset(gate, unit, self.hidden_width, self.hidden_width);
        self.input_weights[input_offset..input_offset + self.input_width]
            .iter()
            .zip(input)
            .map(|(weight, value)| weight * value)
            .chain(
                self.recurrent_weights[recurrent_offset..recurrent_offset + self.hidden_width]
                    .iter()
                    .zip(recurrent_input)
                    .map(|(weight, value)| weight * value),
            )
            .sum::<f64>()
            + self.bias[gate * self.hidden_width + unit]
    }

    pub(crate) fn backward_sequence(
        &self,
        steps: &[GatedRecurrentStep],
        final_hidden_gradient: &[f64],
    ) -> Result<(GatedRecurrentGradients, Vec<Vec<f64>>), TrainableSetError> {
        if final_hidden_gradient.len() != self.hidden_width
            || final_hidden_gradient.iter().any(|value| !value.is_finite())
            || steps.iter().any(|step| {
                step.input.len() != self.input_width
                    || step.previous_hidden.len() != self.hidden_width
                    || step.reset.len() != self.hidden_width
                    || step.update.len() != self.hidden_width
                    || step.candidate.len() != self.hidden_width
                    || step.hidden.len() != self.hidden_width
            })
        {
            return Err(TrainableSetError::new(
                "gated recurrent backward dimensions are invalid",
            ));
        }
        let mut gradients = GatedRecurrentGradients {
            input_weights: vec![0.0; self.input_weights.len()],
            recurrent_weights: vec![0.0; self.recurrent_weights.len()],
            bias: vec![0.0; self.bias.len()],
        };
        let mut input_gradients = vec![vec![0.0; self.input_width]; steps.len()];
        let mut d_hidden = final_hidden_gradient.to_vec();
        for (step_index, step) in steps.iter().enumerate().rev() {
            let mut d_previous = vec![0.0; self.hidden_width];
            let mut d_reset_hidden = vec![0.0; self.hidden_width];
            let mut d_reset = vec![0.0; self.hidden_width];
            let mut d_update = vec![0.0; self.hidden_width];
            let mut d_candidate = vec![0.0; self.hidden_width];
            for unit in 0..self.hidden_width {
                d_previous[unit] += d_hidden[unit] * (1.0 - step.update[unit]);
                d_update[unit] =
                    d_hidden[unit] * (step.candidate[unit] - step.previous_hidden[unit]);
                d_candidate[unit] = d_hidden[unit] * step.update[unit];
            }

            let reset_hidden = step
                .reset
                .iter()
                .zip(&step.previous_hidden)
                .map(|(reset, previous)| reset * previous)
                .collect::<Vec<_>>();
            for (unit, candidate_gradient) in d_candidate.iter().copied().enumerate() {
                let delta = candidate_gradient * (1.0 - step.candidate[unit].powi(2));
                self.accumulate_gate_gradient(
                    CANDIDATE_GATE,
                    unit,
                    delta,
                    &step.input,
                    &reset_hidden,
                    &mut input_gradients[step_index],
                    &mut d_reset_hidden,
                    &mut gradients,
                );
            }
            for unit in 0..self.hidden_width {
                d_reset[unit] += d_reset_hidden[unit] * step.previous_hidden[unit];
                d_previous[unit] += d_reset_hidden[unit] * step.reset[unit];
            }
            for (unit, reset_gradient) in d_reset.iter().copied().enumerate() {
                let delta = reset_gradient * step.reset[unit] * (1.0 - step.reset[unit]);
                self.accumulate_gate_gradient(
                    RESET_GATE,
                    unit,
                    delta,
                    &step.input,
                    &step.previous_hidden,
                    &mut input_gradients[step_index],
                    &mut d_previous,
                    &mut gradients,
                );
            }
            for (unit, update_gradient) in d_update.iter().copied().enumerate() {
                let delta = update_gradient * step.update[unit] * (1.0 - step.update[unit]);
                self.accumulate_gate_gradient(
                    UPDATE_GATE,
                    unit,
                    delta,
                    &step.input,
                    &step.previous_hidden,
                    &mut input_gradients[step_index],
                    &mut d_previous,
                    &mut gradients,
                );
            }
            d_hidden = d_previous;
        }
        Ok((gradients, input_gradients))
    }

    #[allow(clippy::too_many_arguments)]
    fn accumulate_gate_gradient(
        &self,
        gate: usize,
        unit: usize,
        delta: f64,
        input: &[f64],
        recurrent_input: &[f64],
        d_input: &mut [f64],
        d_recurrent_input: &mut [f64],
        gradients: &mut GatedRecurrentGradients,
    ) {
        let input_offset = gate_row_offset(gate, unit, self.hidden_width, self.input_width);
        for feature in 0..self.input_width {
            d_input[feature] += self.input_weights[input_offset + feature] * delta;
            gradients.input_weights[input_offset + feature] += delta * input[feature];
        }
        let recurrent_offset = gate_row_offset(gate, unit, self.hidden_width, self.hidden_width);
        for hidden in 0..self.hidden_width {
            d_recurrent_input[hidden] += self.recurrent_weights[recurrent_offset + hidden] * delta;
            gradients.recurrent_weights[recurrent_offset + hidden] +=
                delta * recurrent_input[hidden];
        }
        gradients.bias[gate * self.hidden_width + unit] += delta;
    }

    pub(crate) fn apply_gradients(
        &mut self,
        gradients: GatedRecurrentGradients,
        learning_rate: f64,
        l2_penalty: f64,
        gradient_clip: f64,
    ) {
        for (weight, gradient) in self
            .input_weights
            .iter_mut()
            .zip(gradients.input_weights)
            .chain(
                self.recurrent_weights
                    .iter_mut()
                    .zip(gradients.recurrent_weights),
            )
        {
            let gradient = gradient + l2_penalty * *weight;
            *weight -= learning_rate * clip(gradient, gradient_clip);
        }
        for (bias, gradient) in self.bias.iter_mut().zip(gradients.bias) {
            *bias -= learning_rate * clip(gradient, gradient_clip);
        }
    }
}

fn gate_row_offset(gate: usize, unit: usize, hidden_width: usize, row_width: usize) -> usize {
    (gate * hidden_width + unit) * row_width
}

fn logistic(value: f64) -> f64 {
    if value >= 0.0 {
        1.0 / (1.0 + (-value).exp())
    } else {
        let exponential = value.exp();
        exponential / (1.0 + exponential)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trainable_set_encoder::dot;

    fn terminal_loss(model: &GatedRecurrent, inputs: &[Vec<f64>], direction: &[f64]) -> f64 {
        let steps = model.forward_sequence(inputs).unwrap();
        let hidden = steps.last().map_or_else(
            || vec![0.0; model.hidden_width()],
            |step| step.hidden.clone(),
        );
        dot(&hidden, direction)
    }

    #[test]
    fn bptt_matches_finite_differences_for_parameters_and_early_input() {
        let mut rng = DeterministicRng::new(0x4752_552d_4752_4144);
        let model = GatedRecurrent::initialized(2, 2, &mut rng).unwrap();
        let inputs = vec![vec![0.2, -0.4], vec![0.5, 0.1]];
        let direction = vec![0.7, -0.3];
        let steps = model.forward_sequence(&inputs).unwrap();
        let (gradients, input_gradients) = model.backward_sequence(&steps, &direction).unwrap();
        let epsilon = 1.0e-6;

        let mut positive = model.clone();
        positive.input_weights[0] += epsilon;
        let mut negative = model.clone();
        negative.input_weights[0] -= epsilon;
        let numeric = (terminal_loss(&positive, &inputs, &direction)
            - terminal_loss(&negative, &inputs, &direction))
            / (2.0 * epsilon);
        assert!((numeric - gradients.input_weights[0]).abs() < 1.0e-6);

        let mut positive = model.clone();
        positive.recurrent_weights[1] += epsilon;
        let mut negative = model.clone();
        negative.recurrent_weights[1] -= epsilon;
        let numeric = (terminal_loss(&positive, &inputs, &direction)
            - terminal_loss(&negative, &inputs, &direction))
            / (2.0 * epsilon);
        assert!((numeric - gradients.recurrent_weights[1]).abs() < 1.0e-6);

        let mut positive_inputs = inputs.clone();
        positive_inputs[0][1] += epsilon;
        let mut negative_inputs = inputs.clone();
        negative_inputs[0][1] -= epsilon;
        let numeric = (terminal_loss(&model, &positive_inputs, &direction)
            - terminal_loss(&model, &negative_inputs, &direction))
            / (2.0 * epsilon);
        assert!((numeric - input_gradients[0][1]).abs() < 1.0e-6);
    }

    #[test]
    fn initialization_and_updates_are_deterministic_bounded_and_finite() {
        let mut first_rng = DeterministicRng::new(17);
        let mut second_rng = DeterministicRng::new(17);
        let mut first = GatedRecurrent::initialized(3, 4, &mut first_rng).unwrap();
        let second = GatedRecurrent::initialized(3, 4, &mut second_rng).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.input_width(), 3);
        assert_eq!(first.hidden_width(), 4);
        assert_eq!(first.parameter_count(), 96);
        assert!(GatedRecurrent::initialized(0, 4, &mut first_rng).is_err());

        let inputs = vec![vec![0.25, 0.5, -0.75], vec![0.0, 0.1, 0.2]];
        let steps = first.forward_sequence(&inputs).unwrap();
        assert!(
            steps
                .last()
                .unwrap()
                .hidden
                .iter()
                .all(|value| (-1.0..=1.0).contains(value))
        );
        let (gradients, _) = first
            .backward_sequence(&steps, &[1.0, -1.0, 0.5, -0.5])
            .unwrap();
        first.apply_gradients(gradients, 0.003, 1.0e-5, 5.0);
        assert_ne!(first, second);
        assert!(first.all_finite());
        assert!(first.forward_sequence(&[vec![0.0; 2]]).is_err());
    }
}
