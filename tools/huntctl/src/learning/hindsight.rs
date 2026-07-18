//! Fail-closed semantic admission for hindsight objective relabeling.
//!
//! Relabeling is permitted only for compiled post-simulation predicates whose
//! truth is a function of one authenticated state snapshot. Rewards must be
//! recomputed from the relabeled predicate; an observed reward is never copied.

use super::goal_conditioning::{CompiledObjectiveVector, GoalConditioningError};
use crate::milestone_dsl::{
    CompiledMilestones, EvaluationPhase, Expression, Field, MilestoneDefinition, decode,
};
use serde::Serialize;
use std::error::Error;
use std::fmt;

pub const HINDSIGHT_RELABEL_DECISION_SCHEMA_V1: &str = "dusklight-hindsight-relabel-decision/v1";

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HindsightRejection {
    PreInputPhase,
    StableHistory,
    SequenceHistory,
    ValueProjection,
    TimelinePosition,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HindsightRelabelDecision {
    pub schema: &'static str,
    pub objective: CompiledObjectiveVector,
    pub eligible: bool,
    pub rejections: Vec<HindsightRejection>,
    pub semantic_class: &'static str,
    pub reward_policy: &'static str,
    pub copied_reward_allowed: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AdmittedHindsightGoal {
    pub objective: CompiledObjectiveVector,
    pub semantic_class: &'static str,
    pub reward_policy: &'static str,
}

impl HindsightRelabelDecision {
    pub fn evaluate(
        compiled: &CompiledMilestones,
        definition_index: usize,
    ) -> Result<Self, HindsightError> {
        let objective = CompiledObjectiveVector::from_compiled(compiled, definition_index)?;
        let decoded = decode(&compiled.bytes)
            .map_err(|error| HindsightError::InvalidProgram(error.to_string()))?;
        let definition = decoded
            .program
            .definitions
            .get(definition_index)
            .ok_or(HindsightError::UnknownDefinition(definition_index))?;
        let rejections = rejection_reasons(definition);
        Ok(Self {
            schema: HINDSIGHT_RELABEL_DECISION_SCHEMA_V1,
            objective,
            eligible: rejections.is_empty(),
            rejections,
            semantic_class: "single_snapshot_post_simulation_predicate",
            reward_policy: "recompute_from_authenticated_pre_post_observations",
            copied_reward_allowed: false,
        })
    }

    pub fn admit(self) -> Result<AdmittedHindsightGoal, HindsightError> {
        if !self.eligible || !self.rejections.is_empty() {
            return Err(HindsightError::Ineligible(self.rejections));
        }
        Ok(AdmittedHindsightGoal {
            objective: self.objective,
            semantic_class: self.semantic_class,
            reward_policy: self.reward_policy,
        })
    }
}

fn rejection_reasons(definition: &MilestoneDefinition) -> Vec<HindsightRejection> {
    let mut reasons = Vec::new();
    if definition.phase != EvaluationPhase::PostSim {
        reasons.push(HindsightRejection::PreInputPhase);
    }
    if definition.stable_ticks != 1 {
        reasons.push(HindsightRejection::StableHistory);
    }
    if !definition.then.is_empty() || definition.within_ticks.is_some() {
        reasons.push(HindsightRejection::SequenceHistory);
    }
    if !definition.projections.is_empty() {
        reasons.push(HindsightRejection::ValueProjection);
    }
    if expression_uses_timeline_position(&definition.when)
        || definition
            .then
            .iter()
            .any(expression_uses_timeline_position)
    {
        reasons.push(HindsightRejection::TimelinePosition);
    }
    reasons
}

fn expression_uses_timeline_position(expression: &Expression) -> bool {
    match expression {
        Expression::Compare { field, .. } => matches!(
            field,
            Field::BoundaryKind | Field::BoundaryIndex | Field::TapeFrame
        ),
        Expression::Query { .. } => false,
        Expression::Not(inner) => expression_uses_timeline_position(inner),
        Expression::And(left, right) | Expression::Or(left, right) => {
            expression_uses_timeline_position(left) || expression_uses_timeline_position(right)
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HindsightError {
    InvalidProgram(String),
    UnknownDefinition(usize),
    InvalidObjective(String),
    Ineligible(Vec<HindsightRejection>),
}

impl fmt::Display for HindsightError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidProgram(message) => {
                write!(formatter, "hindsight program is invalid: {message}")
            }
            Self::UnknownDefinition(index) => {
                write!(formatter, "hindsight definition {index} does not exist")
            }
            Self::InvalidObjective(message) => {
                write!(formatter, "hindsight objective is invalid: {message}")
            }
            Self::Ineligible(reasons) => write!(
                formatter,
                "hindsight objective is semantically ineligible: {reasons:?}"
            ),
        }
    }
}

impl Error for HindsightError {}

impl From<GoalConditioningError> for HindsightError {
    fn from(error: GoalConditioningError) -> Self {
        Self::InvalidObjective(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::milestone_dsl::{
        Comparison, Expression, Field, LanguageVersion, MilestoneDefinition, MilestoneProgram,
        RngStream, Value, ValueProjection, ValueProjectionItem, compile,
    };

    fn compare(field: Field, value: Value) -> Expression {
        Expression::Compare {
            field,
            operator: Comparison::Equal,
            value,
        }
    }

    fn definition(name: &str, expression: Expression) -> MilestoneDefinition {
        MilestoneDefinition {
            name: name.into(),
            phase: EvaluationPhase::PostSim,
            stable_ticks: 1,
            when: expression,
            then: Vec::new(),
            within_ticks: None,
            projections: Vec::new(),
        }
    }

    #[test]
    fn admits_only_state_local_post_simulation_predicates() {
        let eligible = definition("room_one", compare(Field::StageRoom, Value::I32(1)));
        let compiled = compile(&MilestoneProgram {
            version: LanguageVersion { major: 1, minor: 4 },
            definitions: vec![eligible],
        })
        .unwrap();
        let decision = HindsightRelabelDecision::evaluate(&compiled, 0).unwrap();
        assert!(decision.eligible);
        assert!(decision.rejections.is_empty());
        assert!(!decision.copied_reward_allowed);
        assert!(decision.admit().is_ok());
    }

    #[test]
    fn rejects_history_phase_and_timeline_dependent_semantics() {
        let mut pre_input = definition("pre", compare(Field::PlayerExists, Value::Bool(true)));
        pre_input.phase = EvaluationPhase::PreInput;
        let mut stable = definition("stable", compare(Field::PlayerExists, Value::Bool(true)));
        stable.stable_ticks = 2;
        let mut sequence = definition("sequence", compare(Field::StageRoom, Value::I32(1)));
        sequence.then = vec![compare(Field::StageRoom, Value::I32(2))];
        sequence.within_ticks = Some(30);
        let timeline = definition("frame", compare(Field::TapeFrame, Value::U64(100)));
        let mut projection = definition("project", compare(Field::StageRoom, Value::I32(1)));
        projection.projections = vec![ValueProjection {
            name: "rng".into(),
            items: vec![ValueProjectionItem::Rng {
                stream: RngStream::Primary,
            }],
        }];
        let compiled = compile(&MilestoneProgram {
            version: LanguageVersion { major: 1, minor: 4 },
            definitions: vec![pre_input, stable, sequence, timeline, projection],
        })
        .unwrap();

        for (index, reason) in [
            HindsightRejection::PreInputPhase,
            HindsightRejection::StableHistory,
            HindsightRejection::SequenceHistory,
            HindsightRejection::TimelinePosition,
            HindsightRejection::ValueProjection,
        ]
        .into_iter()
        .enumerate()
        {
            let decision = HindsightRelabelDecision::evaluate(&compiled, index).unwrap();
            assert!(!decision.eligible);
            assert!(decision.rejections.contains(&reason));
            assert_eq!(
                decision.admit(),
                Err(HindsightError::Ineligible(vec![reason]))
            );
        }
    }
}
