//! Bounded parameter search and predicate-preserving intervention minimization.

use super::{InterventionOperation, InterventionTape};
use serde::Serialize;
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

pub const MAX_INTERVENTION_SEARCH_AXES: usize = 16;
pub const MAX_INTERVENTION_SEARCH_CANDIDATES: usize = 4_096;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InterventionParameter {
    StartTick,
    DurationTicks,
    VectorComponent { component: usize },
    CurveComponent { point: usize, component: usize },
    FacingYaw,
    Health,
    TimerTicks,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct InterventionParameterAxis {
    pub name: String,
    pub intervention_index: usize,
    pub parameter: InterventionParameter,
    pub minimum: f64,
    pub maximum: f64,
}

#[derive(Clone, Debug)]
pub struct InterventionParameterTemplate {
    seed: InterventionTape,
    axes: Vec<InterventionParameterAxis>,
    initial: Vec<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct InterventionParameterCandidate {
    pub sample_index: usize,
    pub values: Vec<f64>,
    pub tape: InterventionTape,
}

#[derive(Clone, Debug, Serialize)]
pub struct InterventionMinimizationResult {
    pub values: Vec<f64>,
    pub tape: InterventionTape,
    pub evaluations: usize,
}

#[derive(Debug)]
pub struct InterventionParameterError(String);

impl fmt::Display for InterventionParameterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for InterventionParameterError {}

impl InterventionParameterTemplate {
    pub fn new(
        seed: InterventionTape,
        axes: Vec<InterventionParameterAxis>,
    ) -> Result<Self, InterventionParameterError> {
        seed.validate().map_err(parameter_error)?;
        if axes.is_empty() || axes.len() > MAX_INTERVENTION_SEARCH_AXES {
            return Err(error("intervention search requires 1..=16 axes"));
        }
        let mut names = BTreeSet::new();
        let mut keys = BTreeSet::new();
        let mut initial = Vec::with_capacity(axes.len());
        for axis in &axes {
            if axis.name.is_empty()
                || axis.name.len() > 96
                || !names.insert(axis.name.clone())
                || !keys.insert((axis.intervention_index, axis.parameter.clone()))
                || !axis.minimum.is_finite()
                || !axis.maximum.is_finite()
                || axis.minimum >= axis.maximum
            {
                return Err(error(
                    "intervention parameter axes are invalid or duplicated",
                ));
            }
            let value = read_parameter(&seed, axis)?;
            if value < axis.minimum || value > axis.maximum {
                return Err(error("intervention seed lies outside a parameter bound"));
            }
            initial.push(value);
        }
        for (axis_index, axis) in axes.iter().enumerate() {
            for bound in [axis.minimum, axis.maximum] {
                let mut values = initial.clone();
                values[axis_index] = bound;
                candidate_with_values(&seed, &axes, &values)?;
            }
        }
        Ok(Self {
            seed,
            axes,
            initial,
        })
    }

    pub fn initial_values(&self) -> &[f64] {
        &self.initial
    }

    pub fn candidate(
        &self,
        values: &[f64],
    ) -> Result<InterventionTape, InterventionParameterError> {
        candidate_with_values(&self.seed, &self.axes, values)
    }

    /// Generates a deterministic bounded low-discrepancy proposal set. Every
    /// returned proposal has already passed full `DUSKINTR` validation.
    pub fn search_candidates(
        &self,
        budget: usize,
    ) -> Result<Vec<InterventionParameterCandidate>, InterventionParameterError> {
        if budget == 0 || budget > MAX_INTERVENTION_SEARCH_CANDIDATES {
            return Err(error("intervention search budget is outside 1..=4096"));
        }
        let primes = [
            2_u32, 3, 5, 7, 11, 13, 17, 19, 23, 29, 31, 37, 41, 43, 47, 53,
        ];
        let mut seen = BTreeSet::new();
        let mut output = Vec::new();
        for sample_index in 0..budget {
            let values = if sample_index == 0 {
                self.initial.clone()
            } else {
                self.axes
                    .iter()
                    .zip(primes)
                    .map(|(axis, base)| {
                        let unit = radical_inverse(sample_index as u64, base);
                        axis.minimum + unit * (axis.maximum - axis.minimum)
                    })
                    .collect()
            };
            let Ok(tape) = self.candidate(&values) else {
                continue;
            };
            let encoded = tape.encode().map_err(parameter_error)?;
            if seen.insert(encoded) {
                output.push(InterventionParameterCandidate {
                    sample_index,
                    values: realize_values(&self.axes, &values)?,
                    tape,
                });
            }
        }
        if output.is_empty() {
            return Err(error("intervention search produced no valid candidates"));
        }
        Ok(output)
    }

    /// Moves each axis toward zero (or the closest declared endpoint) while a
    /// native causal predicate continues to accept the exact candidate tape.
    pub fn minimize<P>(
        &self,
        mut predicate: P,
    ) -> Result<InterventionMinimizationResult, InterventionParameterError>
    where
        P: FnMut(&InterventionTape) -> Result<bool, InterventionParameterError>,
    {
        let mut values = self.initial.clone();
        let seed = self.candidate(&values)?;
        let mut evaluations = 1;
        if !predicate(&seed)? {
            return Err(error(
                "intervention minimization seed does not preserve the predicate",
            ));
        }
        for axis_index in 0..self.axes.len() {
            let axis = &self.axes[axis_index];
            let neutral = 0.0_f64.clamp(axis.minimum, axis.maximum);
            let mut trial = values.clone();
            trial[axis_index] = neutral;
            if let Ok(candidate) = self.candidate(&trial) {
                evaluations += 1;
                if predicate(&candidate)? {
                    values = realize_values(&self.axes, &trial)?;
                    continue;
                }
            }
            let mut passing = values[axis_index];
            let mut failing = neutral;
            for _ in 0..32 {
                let midpoint = (passing + failing) * 0.5;
                let mut trial = values.clone();
                trial[axis_index] = midpoint;
                let Ok(candidate) = self.candidate(&trial) else {
                    failing = midpoint;
                    continue;
                };
                let realized = realize_value(&axis.parameter, midpoint)?;
                if realized == passing || realized == failing {
                    break;
                }
                evaluations += 1;
                if predicate(&candidate)? {
                    passing = realized;
                    values[axis_index] = realized;
                } else {
                    failing = realized;
                }
            }
        }
        let tape = self.candidate(&values)?;
        Ok(InterventionMinimizationResult {
            values: realize_values(&self.axes, &values)?,
            tape,
            evaluations,
        })
    }
}

fn candidate_with_values(
    seed: &InterventionTape,
    axes: &[InterventionParameterAxis],
    values: &[f64],
) -> Result<InterventionTape, InterventionParameterError> {
    if values.len() != axes.len() {
        return Err(error(
            "intervention parameter width does not match its axes",
        ));
    }
    let mut tape = seed.clone();
    for (axis, value) in axes.iter().zip(values) {
        if !value.is_finite() || *value < axis.minimum || *value > axis.maximum {
            return Err(error("intervention parameter escapes its declared bound"));
        }
        write_parameter(&mut tape, axis, *value)?;
    }
    tape.canonicalize();
    tape.validate().map_err(parameter_error)?;
    Ok(tape)
}

fn realize_values(
    axes: &[InterventionParameterAxis],
    values: &[f64],
) -> Result<Vec<f64>, InterventionParameterError> {
    axes.iter()
        .zip(values)
        .map(|(axis, value)| realize_value(&axis.parameter, *value))
        .collect()
}

fn realize_value(
    parameter: &InterventionParameter,
    value: f64,
) -> Result<f64, InterventionParameterError> {
    match parameter {
        InterventionParameter::StartTick | InterventionParameter::DurationTicks => {
            Ok(f64::from(convert_u32(value)?))
        }
        InterventionParameter::FacingYaw | InterventionParameter::Health => {
            Ok(f64::from(convert_i16(value)?))
        }
        InterventionParameter::TimerTicks => Ok(f64::from(convert_u16(value)?)),
        InterventionParameter::VectorComponent { .. }
        | InterventionParameter::CurveComponent { .. } => Ok(f64::from(value as f32)),
    }
}

fn read_parameter(
    tape: &InterventionTape,
    axis: &InterventionParameterAxis,
) -> Result<f64, InterventionParameterError> {
    let intervention = tape
        .interventions
        .get(axis.intervention_index)
        .ok_or_else(|| error("intervention parameter index is out of range"))?;
    match (&axis.parameter, &intervention.operation) {
        (InterventionParameter::StartTick, _) => Ok(f64::from(intervention.start_tick)),
        (InterventionParameter::DurationTicks, _) => Ok(f64::from(intervention.duration_ticks)),
        (InterventionParameter::VectorComponent { component }, operation) => {
            Ok(f64::from(*vector(operation)?.get(*component).ok_or_else(
                || error("vector component is out of range"),
            )?))
        }
        (
            InterventionParameter::CurveComponent { point, component },
            InterventionOperation::MoveAlongCubicCurve { control_points },
        ) => Ok(f64::from(
            *control_points
                .get(*point)
                .and_then(|value| value.get(*component))
                .ok_or_else(|| error("curve component is out of range"))?,
        )),
        (InterventionParameter::FacingYaw, InterventionOperation::SetFacingYaw { value }) => {
            Ok(f64::from(*value))
        }
        (InterventionParameter::Health, InterventionOperation::SetHealth { value }) => {
            Ok(f64::from(*value))
        }
        (InterventionParameter::TimerTicks, InterventionOperation::SetTimer { ticks, .. }) => {
            Ok(f64::from(*ticks))
        }
        _ => Err(error("intervention parameter does not match its operation")),
    }
}

fn write_parameter(
    tape: &mut InterventionTape,
    axis: &InterventionParameterAxis,
    value: f64,
) -> Result<(), InterventionParameterError> {
    let intervention = tape
        .interventions
        .get_mut(axis.intervention_index)
        .ok_or_else(|| error("intervention parameter index is out of range"))?;
    match (&axis.parameter, &mut intervention.operation) {
        (InterventionParameter::StartTick, _) => intervention.start_tick = convert_u32(value)?,
        (InterventionParameter::DurationTicks, _) => {
            intervention.duration_ticks = convert_u32(value)?
        }
        (InterventionParameter::VectorComponent { component }, operation) => {
            *vector_mut(operation)?
                .get_mut(*component)
                .ok_or_else(|| error("vector component is out of range"))? = value as f32;
        }
        (
            InterventionParameter::CurveComponent { point, component },
            InterventionOperation::MoveAlongCubicCurve { control_points },
        ) => {
            *control_points
                .get_mut(*point)
                .and_then(|value| value.get_mut(*component))
                .ok_or_else(|| error("curve component is out of range"))? = value as f32;
        }
        (
            InterventionParameter::FacingYaw,
            InterventionOperation::SetFacingYaw { value: target },
        ) => *target = convert_i16(value)?,
        (InterventionParameter::Health, InterventionOperation::SetHealth { value: target }) => {
            *target = convert_i16(value)?
        }
        (InterventionParameter::TimerTicks, InterventionOperation::SetTimer { ticks, .. }) => {
            *ticks = convert_u16(value)?
        }
        _ => return Err(error("intervention parameter does not match its operation")),
    }
    Ok(())
}

fn vector(operation: &InterventionOperation) -> Result<&[f32; 3], InterventionParameterError> {
    match operation {
        InterventionOperation::SetPosition { value }
        | InterventionOperation::AddPosition { value }
        | InterventionOperation::SetVelocity { value }
        | InterventionOperation::AddVelocity { value }
        | InterventionOperation::SpawnAtPosition { value } => Ok(value),
        _ => Err(error("operation has no vector magnitude")),
    }
}

fn vector_mut(
    operation: &mut InterventionOperation,
) -> Result<&mut [f32; 3], InterventionParameterError> {
    match operation {
        InterventionOperation::SetPosition { value }
        | InterventionOperation::AddPosition { value }
        | InterventionOperation::SetVelocity { value }
        | InterventionOperation::AddVelocity { value }
        | InterventionOperation::SpawnAtPosition { value } => Ok(value),
        _ => Err(error("operation has no vector magnitude")),
    }
}

fn convert_u32(value: f64) -> Result<u32, InterventionParameterError> {
    let value = value.round();
    if !(0.0..=f64::from(u32::MAX)).contains(&value) {
        return Err(error("parameter does not fit u32"));
    }
    Ok(value as u32)
}

fn convert_u16(value: f64) -> Result<u16, InterventionParameterError> {
    let value = value.round();
    if !(0.0..=f64::from(u16::MAX)).contains(&value) {
        return Err(error("parameter does not fit u16"));
    }
    Ok(value as u16)
}

fn convert_i16(value: f64) -> Result<i16, InterventionParameterError> {
    let value = value.round();
    if value < f64::from(i16::MIN) || value > f64::from(i16::MAX) {
        return Err(error("parameter does not fit i16"));
    }
    Ok(value as i16)
}

fn radical_inverse(mut index: u64, base: u32) -> f64 {
    let mut factor = 1.0 / f64::from(base);
    let mut value = 0.0;
    while index != 0 {
        value += f64::from((index % u64::from(base)) as u32) * factor;
        index /= u64::from(base);
        factor /= f64::from(base);
    }
    value
}

fn parameter_error(error: impl fmt::Display) -> InterventionParameterError {
    InterventionParameterError(error.to_string())
}

fn error(message: impl Into<String>) -> InterventionParameterError {
    InterventionParameterError(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn template() -> InterventionParameterTemplate {
        let tape = InterventionTape::compile_dsl(
            "timeline 20\nat 5 for 3 before_game_tick process 7 require actor_exists add_velocity 8 0 0",
        )
        .unwrap();
        InterventionParameterTemplate::new(
            tape,
            vec![
                InterventionParameterAxis {
                    name: "start".into(),
                    intervention_index: 0,
                    parameter: InterventionParameter::StartTick,
                    minimum: 0.0,
                    maximum: 10.0,
                },
                InterventionParameterAxis {
                    name: "duration".into(),
                    intervention_index: 0,
                    parameter: InterventionParameter::DurationTicks,
                    minimum: 1.0,
                    maximum: 5.0,
                },
                InterventionParameterAxis {
                    name: "velocity_x".into(),
                    intervention_index: 0,
                    parameter: InterventionParameter::VectorComponent { component: 0 },
                    minimum: 0.0,
                    maximum: 8.0,
                },
            ],
        )
        .unwrap()
    }

    #[test]
    fn bounded_search_is_deterministic_canonical_and_deduplicated() {
        let template = template();
        let first = template.search_candidates(64).unwrap();
        let second = template.search_candidates(64).unwrap();
        assert_eq!(first.len(), second.len());
        assert_eq!(
            first
                .iter()
                .map(|candidate| candidate.tape.encode().unwrap())
                .collect::<Vec<_>>(),
            second
                .iter()
                .map(|candidate| candidate.tape.encode().unwrap())
                .collect::<Vec<_>>()
        );
        assert!(first.len() > 8);
        assert!(
            first
                .iter()
                .all(|candidate| candidate.tape.validate().is_ok())
        );
    }

    #[test]
    fn minimization_preserves_exact_causal_predicate_while_reducing_axes() {
        let result = template()
            .minimize(|tape| {
                let intervention = &tape.interventions[0];
                let velocity_x = match intervention.operation {
                    InterventionOperation::AddVelocity { value } => value[0],
                    _ => return Err(error("unexpected operation")),
                };
                Ok(intervention.start_tick >= 3
                    && intervention.duration_ticks >= 2
                    && velocity_x >= 2.0)
            })
            .unwrap();
        assert_eq!(result.values[0], 3.0);
        assert_eq!(result.values[1], 2.0);
        assert!((result.values[2] - 2.0).abs() < 0.000_01);
        assert!(result.evaluations > 3);
        result.tape.validate().unwrap();
    }

    #[test]
    fn axes_must_match_typed_operations_and_valid_endpoints() {
        let tape = InterventionTape::compile_dsl(
            "timeline 4\nat 1 for 1 before_game_tick process 7 require actor_exists set_health 3",
        )
        .unwrap();
        assert!(
            InterventionParameterTemplate::new(
                tape,
                vec![InterventionParameterAxis {
                    name: "wrong".into(),
                    intervention_index: 0,
                    parameter: InterventionParameter::VectorComponent { component: 0 },
                    minimum: 0.0,
                    maximum: 2.0,
                }],
            )
            .is_err()
        );
    }

    #[test]
    fn timing_search_can_reorder_a_target_and_still_emit_canonical_tape() {
        let tape = InterventionTape::compile_dsl(
            "timeline 20\nat 5 for 1 before_game_tick process 7 require actor_exists set_health 1\nat 8 for 1 before_game_tick process 8 require actor_exists set_health 2",
        )
        .unwrap();
        let template = InterventionParameterTemplate::new(
            tape,
            vec![InterventionParameterAxis {
                name: "second_start".into(),
                intervention_index: 1,
                parameter: InterventionParameter::StartTick,
                minimum: 1.0,
                maximum: 8.0,
            }],
        )
        .unwrap();
        let candidate = template.candidate(&[1.0]).unwrap();
        assert_eq!(candidate.interventions[0].start_tick, 1);
        assert!(matches!(
            candidate.interventions[0].operation,
            InterventionOperation::SetHealth { value: 2 }
        ));
        candidate.validate().unwrap();
        assert!(
            template
                .search_candidates(MAX_INTERVENTION_SEARCH_CANDIDATES + 1)
                .is_err()
        );
    }
}
