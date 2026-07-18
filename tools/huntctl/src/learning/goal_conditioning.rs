//! Fixed-width goal vectors derived from canonical compiled milestone programs.
//!
//! A caller selects one compiled definition. The vector retains its exact
//! digest and bounded semantic structure, so a value or policy model can share
//! one input layout across objectives without accepting a free-form segment ID.

use crate::artifact::Digest;
use crate::milestone_dsl::{
    Comparison, CompiledMilestones, EvaluationPhase, Expression, MAX_OPS, MAX_PROJECTIONS,
    MilestoneDefinition, QueryFact, Value, decode,
};
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fmt;

pub const COMPILED_OBJECTIVE_VECTOR_SCHEMA_V1: &str = "dusklight-compiled-objective-vector/v1";
pub const COMPILED_OBJECTIVE_VECTOR_WIDTH: usize = 64;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledObjectiveVector {
    pub schema: String,
    pub program_sha256: Digest,
    pub definition_sha256: Digest,
    pub definition_name: String,
    pub values: Vec<f32>,
}

impl CompiledObjectiveVector {
    pub fn from_compiled(
        compiled: &CompiledMilestones,
        definition_index: usize,
    ) -> Result<Self, GoalConditioningError> {
        let decoded = decode(&compiled.bytes)
            .map_err(|error| GoalConditioningError::InvalidProgram(error.to_string()))?;
        if decoded.program_sha256 != compiled.program_sha256
            || decoded.definitions != compiled.definitions
        {
            return Err(GoalConditioningError::InvalidProgram(
                "compiled milestone identity differs from canonical bytes".into(),
            ));
        }
        let definition = decoded
            .program
            .definitions
            .get(definition_index)
            .ok_or(GoalConditioningError::UnknownDefinition(definition_index))?;
        let identity = decoded
            .definitions
            .get(definition_index)
            .ok_or(GoalConditioningError::UnknownDefinition(definition_index))?;
        let vector = Self {
            schema: COMPILED_OBJECTIVE_VECTOR_SCHEMA_V1.into(),
            program_sha256: Digest(decoded.program_sha256),
            definition_sha256: Digest(identity.sha256),
            definition_name: identity.name.clone(),
            values: encode_definition(identity.sha256, definition),
        };
        vector.validate()?;
        Ok(vector)
    }

    pub fn validate(&self) -> Result<(), GoalConditioningError> {
        if self.schema != COMPILED_OBJECTIVE_VECTOR_SCHEMA_V1
            || self.program_sha256 == Digest::ZERO
            || self.definition_sha256 == Digest::ZERO
            || self.definition_name.is_empty()
            || self.values.len() != COMPILED_OBJECTIVE_VECTOR_WIDTH
            || self.values.iter().any(|value| !value.is_finite())
        {
            return Err(GoalConditioningError::InvalidVector);
        }
        Ok(())
    }
}

fn encode_definition(identity: [u8; 32], definition: &MilestoneDefinition) -> Vec<f32> {
    let mut values = Vec::with_capacity(COMPILED_OBJECTIVE_VECTOR_WIDTH);
    values.extend(identity.map(|byte| f32::from(byte) / 127.5 - 1.0));
    values.extend(match definition.phase {
        EvaluationPhase::PreInput => [1.0, 0.0],
        EvaluationPhase::PostSim => [0.0, 1.0],
    });
    values.push(f32::from(definition.stable_ticks) / f32::from(u16::MAX));
    values.push(definition.then.len() as f32 / MAX_OPS as f32);
    values.push(f32::from(definition.within_ticks.is_some()));
    values.push(
        definition
            .within_ticks
            .map_or(0.0, |ticks| f32::from(ticks) / f32::from(u16::MAX)),
    );
    values.push(definition.projections.len() as f32 / MAX_PROJECTIONS as f32);

    let mut statistics = ObjectiveStatistics::default();
    statistics.visit(&definition.when);
    for expression in &definition.then {
        statistics.visit(expression);
    }
    let scale = MAX_OPS as f32;
    values.extend(statistics.expressions.map(|count| count as f32 / scale));
    values.extend(statistics.queries.map(|count| count as f32 / scale));
    values.extend(statistics.comparisons.map(|count| count as f32 / scale));
    values.extend(statistics.values.map(|count| count as f32 / scale));
    debug_assert_eq!(values.len(), COMPILED_OBJECTIVE_VECTOR_WIDTH);
    values
}

#[derive(Default)]
struct ObjectiveStatistics {
    expressions: [u16; 5],
    queries: [u16; 4],
    comparisons: [u16; 8],
    values: [u16; 8],
}

impl ObjectiveStatistics {
    fn visit(&mut self, expression: &Expression) {
        match expression {
            Expression::Compare {
                operator, value, ..
            } => {
                self.expressions[0] += 1;
                self.comparisons[comparison_index(*operator)] += 1;
                self.values[value_index(value)] += 1;
            }
            Expression::Query {
                fact,
                operator,
                value,
            } => {
                self.expressions[1] += 1;
                self.queries[query_index(fact)] += 1;
                self.comparisons[comparison_index(*operator)] += 1;
                self.values[value_index(value)] += 1;
            }
            Expression::Not(inner) => {
                self.expressions[2] += 1;
                self.visit(inner);
            }
            Expression::And(left, right) => {
                self.expressions[3] += 1;
                self.visit(left);
                self.visit(right);
            }
            Expression::Or(left, right) => {
                self.expressions[4] += 1;
                self.visit(left);
                self.visit(right);
            }
        }
    }
}

const fn query_index(fact: &QueryFact) -> usize {
    match fact {
        QueryFact::PlacedActor { .. } => 0,
        QueryFact::Flag { .. } => 1,
        QueryFact::PlayerInAabb { .. } => 2,
        QueryFact::PlayerPlaneSignedDistance { .. } => 3,
    }
}

const fn comparison_index(comparison: Comparison) -> usize {
    match comparison {
        Comparison::Equal => 0,
        Comparison::NotEqual => 1,
        Comparison::Less => 2,
        Comparison::LessEqual => 3,
        Comparison::Greater => 4,
        Comparison::GreaterEqual => 5,
        Comparison::HasAll => 6,
        Comparison::HasAny => 7,
    }
}

const fn value_index(value: &Value) -> usize {
    match value {
        Value::Bool(_) => 0,
        Value::U32(_) => 1,
        Value::U64(_) => 2,
        Value::I32(_) => 3,
        Value::F32(_) => 4,
        Value::Symbol(_) => 5,
        Value::ProcedureNumber(_) => 6,
        Value::ProcedureSymbol(_) => 7,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GoalConditioningError {
    InvalidProgram(String),
    UnknownDefinition(usize),
    InvalidVector,
}

impl fmt::Display for GoalConditioningError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidProgram(message) => {
                write!(formatter, "compiled objective is invalid: {message}")
            }
            Self::UnknownDefinition(index) => write!(
                formatter,
                "compiled objective definition {index} does not exist"
            ),
            Self::InvalidVector => formatter.write_str("compiled objective vector is invalid"),
        }
    }
}

impl Error for GoalConditioningError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::milestone_dsl::{compile_source, parse};

    const SOURCE: &str = r#"milestones 1.0
milestone room_one {
  phase post_sim
  when stage.room == 1
}
milestone fast_player {
  phase post_sim
  stable 2
  when player.speed > 3.0
}
"#;

    #[test]
    fn exact_compiled_definitions_produce_distinct_fixed_width_vectors() {
        let compiled = compile_source(SOURCE).unwrap();
        let room = CompiledObjectiveVector::from_compiled(&compiled, 0).unwrap();
        let speed = CompiledObjectiveVector::from_compiled(&compiled, 1).unwrap();
        assert_eq!(room.values.len(), COMPILED_OBJECTIVE_VECTOR_WIDTH);
        assert_ne!(room.definition_sha256, speed.definition_sha256);
        assert_ne!(room.values, speed.values);
        assert_eq!(room.program_sha256, speed.program_sha256);
        assert_eq!(room.definition_name, "room_one");
    }

    #[test]
    fn vector_is_derived_from_canonical_bytes_not_detached_source_state() {
        let compiled = compile_source(SOURCE).unwrap();
        let first = CompiledObjectiveVector::from_compiled(&compiled, 0).unwrap();
        let mut parsed = parse(SOURCE).unwrap();
        parsed.definitions[0].stable_ticks = 3;
        let changed = crate::milestone_dsl::compile(&parsed).unwrap();
        let second = CompiledObjectiveVector::from_compiled(&changed, 0).unwrap();
        assert_ne!(first.definition_sha256, second.definition_sha256);
        assert_ne!(first.values, second.values);

        let mut tampered = compiled;
        tampered.bytes[20] ^= 1;
        assert!(CompiledObjectiveVector::from_compiled(&tampered, 0).is_err());
    }
}
