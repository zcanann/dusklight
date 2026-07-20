//! Fixed-width goal vectors derived from canonical compiled milestone programs.
//!
//! A caller selects one compiled definition. The vector retains its exact
//! digest and bounded semantic structure, so a value or policy model can share
//! one input layout across objectives without accepting a free-form segment ID.

use super::factorized_actions::{
    FactorizedActionEncoder, FactorizedActionError, FactorizedOptionAction,
};
use crate::artifact::Digest;
use crate::milestone_dsl::{
    Comparison, CompiledMilestones, EvaluationPhase, Expression, MAX_OPS, MAX_PROJECTIONS,
    MilestoneDefinition, QueryFact, Value, decode,
};
use crate::transition_corpus::MAX_FEATURES;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;

pub const COMPILED_OBJECTIVE_VECTOR_SCHEMA_V2: &str = "dusklight-compiled-objective-vector/v2";
pub const COMPILED_OBJECTIVE_VECTOR_WIDTH: usize = 65;
pub const GOAL_CONDITIONED_INPUT_SCHEMA_V2: &str = "dusklight-goal-conditioned-input/v2";

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledObjectiveVector {
    pub schema: String,
    pub program_sha256: Digest,
    pub definition_sha256: Digest,
    pub definition_name: String,
    pub values: Vec<f32>,
    pub vector_sha256: Digest,
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
        let values = encode_definition(identity.sha256, definition);
        let vector = Self {
            schema: COMPILED_OBJECTIVE_VECTOR_SCHEMA_V2.into(),
            program_sha256: Digest(decoded.program_sha256),
            definition_sha256: Digest(identity.sha256),
            definition_name: identity.name.clone(),
            vector_sha256: vector_digest(
                Digest(decoded.program_sha256),
                Digest(identity.sha256),
                &identity.name,
                &values,
            ),
            values,
        };
        vector.validate()?;
        Ok(vector)
    }

    pub fn validate(&self) -> Result<(), GoalConditioningError> {
        if self.schema != COMPILED_OBJECTIVE_VECTOR_SCHEMA_V2
            || self.program_sha256 == Digest::ZERO
            || self.definition_sha256 == Digest::ZERO
            || self.definition_name.is_empty()
            || self.values.len() != COMPILED_OBJECTIVE_VECTOR_WIDTH
            || self.values.iter().any(|value| !value.is_finite())
            || self.vector_sha256
                != vector_digest(
                    self.program_sha256,
                    self.definition_sha256,
                    &self.definition_name,
                    &self.values,
                )
        {
            return Err(GoalConditioningError::InvalidVector);
        }
        Ok(())
    }
}

fn vector_digest(program: Digest, definition: Digest, name: &str, values: &[f32]) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight.compiled-objective-vector.identity/v2\0");
    hasher.update(program.0);
    hasher.update(definition.0);
    hasher.update((name.len() as u64).to_le_bytes());
    hasher.update(name.as_bytes());
    for value in values {
        hasher.update(value.to_bits().to_le_bytes());
    }
    Digest(hasher.finalize().into())
}

/// Authenticated input layout shared by option value and behavior-policy models.
#[derive(Clone, Debug, Serialize)]
pub struct GoalConditionedInputEncoder {
    schema: &'static str,
    state_schema_sha256: Digest,
    state_width: usize,
    action_factor_schema_sha256: Digest,
    action_encoder: FactorizedActionEncoder,
    objective_width: usize,
    policy_layout: &'static str,
    value_layout: &'static str,
}

impl GoalConditionedInputEncoder {
    pub fn new(
        state_schema_sha256: Digest,
        state_width: usize,
        action_encoder: FactorizedActionEncoder,
    ) -> Result<Self, GoalConditioningError> {
        if state_schema_sha256 == Digest::ZERO || state_width == 0 || state_width > MAX_FEATURES {
            return Err(GoalConditioningError::InvalidInputLayout);
        }
        let action_factor_schema_sha256 = action_encoder
            .schema_sha256()
            .map_err(GoalConditioningError::from)?;
        Ok(Self {
            schema: GOAL_CONDITIONED_INPUT_SCHEMA_V2,
            state_schema_sha256,
            state_width,
            action_factor_schema_sha256,
            action_encoder,
            objective_width: COMPILED_OBJECTIVE_VECTOR_WIDTH,
            policy_layout: "state_then_compiled_objective",
            value_layout: "state_then_compiled_objective_then_action_factors",
        })
    }

    pub fn policy_input(
        &self,
        state: &[f32],
        objective: &CompiledObjectiveVector,
    ) -> Result<Vec<f64>, GoalConditioningError> {
        self.validate_state_and_objective(state, objective)?;
        Ok(state
            .iter()
            .map(|value| f64::from(*value))
            .chain(objective.values.iter().map(|value| f64::from(*value)))
            .collect())
    }

    pub fn value_input(
        &self,
        state: &[f32],
        objective: &CompiledObjectiveVector,
        action: &FactorizedOptionAction,
    ) -> Result<Vec<f64>, GoalConditioningError> {
        let mut input = self.policy_input(state, objective)?;
        input.extend(
            self.action_encoder
                .encode(action)
                .map_err(GoalConditioningError::from)?,
        );
        Ok(input)
    }

    pub fn policy_width(&self) -> usize {
        self.state_width + self.objective_width
    }

    pub fn value_width(&self) -> usize {
        self.policy_width() + self.action_encoder.feature_width()
    }

    pub fn schema_sha256(&self) -> Result<Digest, GoalConditioningError> {
        let bytes = serde_json::to_vec(self)
            .map_err(|error| GoalConditioningError::Serialization(error.to_string()))?;
        Ok(Digest(Sha256::digest(bytes).into()))
    }

    fn validate_state_and_objective(
        &self,
        state: &[f32],
        objective: &CompiledObjectiveVector,
    ) -> Result<(), GoalConditioningError> {
        if state.len() != self.state_width {
            return Err(GoalConditioningError::FeatureWidth {
                expected: self.state_width,
                actual: state.len(),
            });
        }
        if state.iter().any(|value| !value.is_finite()) {
            return Err(GoalConditioningError::NonFiniteState);
        }
        objective.validate()?;
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
    queries: [u16; 5],
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
        QueryFact::TemporaryEventByte { .. } => 4,
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
    InvalidInputLayout,
    FeatureWidth { expected: usize, actual: usize },
    NonFiniteState,
    ActionFactors(String),
    Serialization(String),
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
            Self::InvalidInputLayout => {
                formatter.write_str("goal-conditioned input layout is invalid")
            }
            Self::FeatureWidth { expected, actual } => write!(
                formatter,
                "goal-conditioned state width is {actual}, expected {expected}"
            ),
            Self::NonFiniteState => {
                formatter.write_str("goal-conditioned state contains a non-finite value")
            }
            Self::ActionFactors(message) => write!(
                formatter,
                "goal-conditioned action factors are invalid: {message}"
            ),
            Self::Serialization(message) => write!(
                formatter,
                "goal-conditioned schema serialization failed: {message}"
            ),
        }
    }
}

impl Error for GoalConditioningError {}

impl From<FactorizedActionError> for GoalConditioningError {
    fn from(error: FactorizedActionError) -> Self {
        Self::ActionFactors(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::super::factorized_actions::{FactorizedActionEncoder, FactorizedOptionAction};
    use super::*;
    use crate::milestone_dsl::{compile_source, parse};
    use crate::option_execution::OptionType;

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
        assert_ne!(room.vector_sha256, Digest::ZERO);
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

        let mut tampered_vector = first;
        tampered_vector.values[40] += 1.0;
        assert_eq!(
            tampered_vector.validate(),
            Err(GoalConditioningError::InvalidVector)
        );
    }

    #[test]
    fn one_layout_conditions_policy_and_value_inputs_on_compiled_goals() {
        let compiled = compile_source(SOURCE).unwrap();
        let room = CompiledObjectiveVector::from_compiled(&compiled, 0).unwrap();
        let speed = CompiledObjectiveVector::from_compiled(&compiled, 1).unwrap();
        let neutral = FactorizedOptionAction::new(OptionType::Neutral, 1);
        let roll = FactorizedOptionAction::new(OptionType::Roll, 4);
        let action_encoder = FactorizedActionEncoder::fit([&neutral, &roll]).unwrap();
        let encoder = GoalConditionedInputEncoder::new(Digest([7; 32]), 2, action_encoder).unwrap();

        let state = [1.0, 2.0];
        let room_policy = encoder.policy_input(&state, &room).unwrap();
        let speed_policy = encoder.policy_input(&state, &speed).unwrap();
        assert_eq!(room_policy.len(), encoder.policy_width());
        assert_ne!(room_policy, speed_policy);
        assert_eq!(&room_policy[..2], &[1.0, 2.0]);

        let neutral_value = encoder.value_input(&state, &room, &neutral).unwrap();
        let roll_value = encoder.value_input(&state, &room, &roll).unwrap();
        assert_eq!(neutral_value.len(), encoder.value_width());
        assert_eq!(&neutral_value[..encoder.policy_width()], &room_policy);
        assert_ne!(neutral_value, roll_value);
        assert_ne!(encoder.schema_sha256().unwrap(), Digest::ZERO);
    }
}
