//! Fail-closed semantic admission for hindsight objective relabeling.
//!
//! Relabeling is permitted only for compiled post-simulation predicates whose
//! truth is a function of one authenticated state snapshot. Rewards must be
//! recomputed from the relabeled predicate; an observed reward is never copied.

use super::goal_conditioning::{CompiledObjectiveVector, GoalConditioningError};
use super::option_values::{
    OptionValueBatch, OptionValueConfig, OptionValueModel, OptionValueSample,
};
use crate::artifact::Digest;
use crate::milestone_dsl::{
    CompiledMilestones, EvaluationPhase, Expression, Field, MilestoneDefinition, decode,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

pub const HINDSIGHT_RELABEL_DECISION_SCHEMA_V1: &str = "dusklight-hindsight-relabel-decision/v1";
pub const HINDSIGHT_PREDICATE_EVIDENCE_SCHEMA_V1: &str =
    "dusklight-hindsight-predicate-evidence/v1";
pub const HINDSIGHT_RELABELED_OPTION_SCHEMA_V1: &str = "dusklight-hindsight-relabeled-option/v1";

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

/// Native predicate results bound to the exact compact transition and raw tape.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HindsightPredicateEvidence {
    pub schema: String,
    pub objective_vector_sha256: Digest,
    pub program_sha256: Digest,
    pub definition_sha256: Digest,
    pub pre_observation_sha256: Digest,
    pub post_observation_sha256: Digest,
    pub state_sha256: Digest,
    pub next_state_sha256: Digest,
    pub realized_tape_sha256: Digest,
    pub pre_satisfied: bool,
    pub post_satisfied: bool,
    pub evaluator: String,
}

impl HindsightPredicateEvidence {
    pub fn bind(
        goal: &AdmittedHindsightGoal,
        transition: &OptionValueSample,
        pre_observation_sha256: Digest,
        post_observation_sha256: Digest,
        pre_satisfied: bool,
        post_satisfied: bool,
    ) -> Result<Self, HindsightError> {
        let evidence = Self {
            schema: HINDSIGHT_PREDICATE_EVIDENCE_SCHEMA_V1.into(),
            objective_vector_sha256: goal.objective.vector_sha256,
            program_sha256: goal.objective.program_sha256,
            definition_sha256: goal.objective.definition_sha256,
            pre_observation_sha256,
            post_observation_sha256,
            state_sha256: state_digest(&transition.state),
            next_state_sha256: state_digest(&transition.next_state),
            realized_tape_sha256: transition.realized_tape_sha256,
            pre_satisfied,
            post_satisfied,
            evaluator: "native_compiled_milestone_pre_post".into(),
        };
        evidence.validate(goal, transition)?;
        Ok(evidence)
    }

    fn validate(
        &self,
        goal: &AdmittedHindsightGoal,
        transition: &OptionValueSample,
    ) -> Result<(), HindsightError> {
        if self.schema != HINDSIGHT_PREDICATE_EVIDENCE_SCHEMA_V1
            || self.objective_vector_sha256 != goal.objective.vector_sha256
            || self.program_sha256 != goal.objective.program_sha256
            || self.definition_sha256 != goal.objective.definition_sha256
            || self.pre_observation_sha256 == Digest::ZERO
            || self.post_observation_sha256 == Digest::ZERO
            || self.pre_observation_sha256 == self.post_observation_sha256
            || self.state_sha256 != state_digest(&transition.state)
            || self.next_state_sha256 != state_digest(&transition.next_state)
            || self.realized_tape_sha256 != transition.realized_tape_sha256
            || self.realized_tape_sha256 == Digest::ZERO
            || self.pre_satisfied
            || !self.post_satisfied
            || self.evaluator != "native_compiled_milestone_pre_post"
        {
            return Err(HindsightError::InvalidEvidence);
        }
        Ok(())
    }
}

/// Auditable output: the original reward is retained, while training consumes
/// the independently configured relabeled achievement reward.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RelabeledHindsightOption {
    pub schema: &'static str,
    pub objective: CompiledObjectiveVector,
    pub transition: OptionValueSample,
    pub original_reward: f32,
    pub reward_recomputed: bool,
    pub evidence: HindsightPredicateEvidence,
    pub promotion_authority: bool,
}

impl RelabeledHindsightOption {
    pub fn validate(&self) -> Result<(), HindsightError> {
        self.objective.validate()?;
        let goal = AdmittedHindsightGoal {
            objective: self.objective.clone(),
            semantic_class: "single_snapshot_post_simulation_predicate",
            reward_policy: "recompute_from_authenticated_pre_post_observations",
        };
        self.evidence.validate(&goal, &self.transition)?;
        if self.schema != HINDSIGHT_RELABELED_OPTION_SCHEMA_V1
            || !self.original_reward.is_finite()
            || !self.transition.reward.is_finite()
            || !self.transition.terminal
            || !self.reward_recomputed
            || self.promotion_authority
        {
            return Err(HindsightError::InvalidRelabeledTransition);
        }
        Ok(())
    }
}

/// Objective-separated replay for hindsight rows.
///
/// Relabeled samples never enter the primary objective's batch. Each compiled
/// objective gets its own option-value model, preventing an achieved alternate
/// goal from changing the native terminal authority or targets of another.
#[derive(Debug)]
pub struct HindsightOptionReplay {
    feature_schema: Digest,
    rows: Vec<RelabeledHindsightOption>,
    episode_groups: Vec<u64>,
    models: BTreeMap<Digest, OptionValueModel>,
}

impl HindsightOptionReplay {
    pub fn new(feature_schema: Digest) -> Result<Self, HindsightError> {
        if feature_schema == Digest::ZERO {
            return Err(HindsightError::InvalidFeatureSchema);
        }
        Ok(Self {
            feature_schema,
            rows: Vec::new(),
            episode_groups: Vec::new(),
            models: BTreeMap::new(),
        })
    }

    pub fn rows(&self) -> &[RelabeledHindsightOption] {
        &self.rows
    }

    pub fn model(&self, objective_sha256: Digest) -> Option<&OptionValueModel> {
        self.models.get(&objective_sha256)
    }

    pub fn admit_and_refit(
        &mut self,
        row: RelabeledHindsightOption,
        episode_group: u64,
        config: &OptionValueConfig,
    ) -> Result<&OptionValueModel, HindsightError> {
        row.validate()?;
        if self.rows.iter().any(|existing| {
            existing.objective.definition_sha256 == row.objective.definition_sha256
                && existing.evidence.pre_observation_sha256 == row.evidence.pre_observation_sha256
                && existing.evidence.post_observation_sha256 == row.evidence.post_observation_sha256
                && existing.transition.realized_tape_sha256 == row.transition.realized_tape_sha256
        }) {
            return Err(HindsightError::DuplicateRelabeledTransition);
        }
        let objective_sha256 = row.objective.definition_sha256;
        let mut samples = Vec::new();
        let mut groups = Vec::new();
        for (existing, group) in self.rows.iter().zip(&self.episode_groups) {
            if existing.objective.definition_sha256 == objective_sha256 {
                samples.push(existing.transition.clone());
                groups.push(*group);
            }
        }
        samples.push(row.transition.clone());
        groups.push(episode_group);
        let feature_width = samples[0].state.len();
        let batch = OptionValueBatch::new(
            self.feature_schema,
            objective_sha256,
            feature_width,
            samples,
            groups,
        )
        .map_err(|error| HindsightError::OptionValues(error.to_string()))?;
        let model = OptionValueModel::fit_batch(&batch, config)
            .map_err(|error| HindsightError::OptionValues(error.to_string()))?;

        self.rows.push(row);
        self.episode_groups.push(episode_group);
        self.models.insert(objective_sha256, model);
        Ok(self.models.get(&objective_sha256).unwrap())
    }
}

impl AdmittedHindsightGoal {
    pub fn relabel_transition(
        &self,
        transition: &OptionValueSample,
        evidence: HindsightPredicateEvidence,
        achievement_reward: f32,
    ) -> Result<RelabeledHindsightOption, HindsightError> {
        self.objective.validate()?;
        evidence.validate(self, transition)?;
        if !achievement_reward.is_finite() {
            return Err(HindsightError::InvalidReward);
        }
        let original_reward = transition.reward;
        let mut relabeled = transition.clone();
        relabeled.reward = achievement_reward;
        relabeled.terminal = true;
        Ok(RelabeledHindsightOption {
            schema: HINDSIGHT_RELABELED_OPTION_SCHEMA_V1,
            objective: self.objective.clone(),
            transition: relabeled,
            original_reward,
            reward_recomputed: true,
            evidence,
            promotion_authority: false,
        })
    }
}

fn state_digest(state: &[f32]) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight.hindsight.compact-state/v1\0");
    hasher.update((state.len() as u64).to_le_bytes());
    for value in state {
        hasher.update(value.to_bits().to_le_bytes());
    }
    Digest(hasher.finalize().into())
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
    InvalidEvidence,
    InvalidReward,
    InvalidRelabeledTransition,
    InvalidFeatureSchema,
    DuplicateRelabeledTransition,
    OptionValues(String),
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
            Self::InvalidEvidence => formatter.write_str(
                "hindsight predicate evidence is stale, detached, or not a false-to-true hit",
            ),
            Self::InvalidReward => {
                formatter.write_str("hindsight achievement reward must be finite")
            }
            Self::InvalidRelabeledTransition => {
                formatter.write_str("hindsight relabeled transition is invalid or authoritative")
            }
            Self::InvalidFeatureSchema => {
                formatter.write_str("hindsight replay feature schema is zero")
            }
            Self::DuplicateRelabeledTransition => {
                formatter.write_str("hindsight replay already contains this native transition")
            }
            Self::OptionValues(message) => {
                write!(formatter, "hindsight option-value replay failed: {message}")
            }
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
    use super::super::option_values::OptionActionDescriptor;
    use super::*;
    use crate::milestone_dsl::{
        Comparison, Expression, Field, LanguageVersion, MilestoneDefinition, MilestoneProgram,
        RngStream, Value, ValueProjection, ValueProjectionItem, compile,
    };
    use crate::option_execution::OptionType;
    use std::collections::BTreeMap;

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

    fn option_sample() -> OptionValueSample {
        OptionValueSample {
            action: OptionActionDescriptor {
                option_id: "move".into(),
                option_type: OptionType::Move,
                parameters: BTreeMap::new(),
            },
            state: vec![0.0],
            duration_ticks: 4,
            reward: -0.25,
            next_state: vec![1.0],
            terminal: false,
            before_state_sha256: Digest([4; 32]),
            after_state_sha256: Digest([5; 32]),
            source_checkpoint_sha256: Digest([6; 32]),
            next_checkpoint_sha256: Digest([7; 32]),
            realized_tape_range: crate::option_execution::TapeRange {
                start_frame: 0,
                end_frame_exclusive: 4,
            },
            realized_tape_sha256: Digest([3; 32]),
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

    #[test]
    fn relabel_requires_bound_false_to_true_native_evidence_and_recomputes_reward() {
        let compiled = compile(&MilestoneProgram {
            version: LanguageVersion { major: 1, minor: 4 },
            definitions: vec![definition(
                "room_one",
                compare(Field::StageRoom, Value::I32(1)),
            )],
        })
        .unwrap();
        let goal = HindsightRelabelDecision::evaluate(&compiled, 0)
            .unwrap()
            .admit()
            .unwrap();
        let transition = option_sample();
        let evidence = HindsightPredicateEvidence::bind(
            &goal,
            &transition,
            Digest([4; 32]),
            Digest([5; 32]),
            false,
            true,
        )
        .unwrap();
        let relabeled = goal
            .relabel_transition(&transition, evidence, 10.0)
            .unwrap();
        assert_eq!(relabeled.original_reward, -0.25);
        assert_eq!(relabeled.transition.reward, 10.0);
        assert!(relabeled.transition.terminal);
        assert!(relabeled.reward_recomputed);
        assert!(!relabeled.promotion_authority);
        relabeled.validate().unwrap();

        let objective_sha256 = relabeled.objective.definition_sha256;
        let mut replay = HindsightOptionReplay::new(Digest([9; 32])).unwrap();
        replay
            .admit_and_refit(relabeled.clone(), 12, &OptionValueConfig::default())
            .unwrap();
        assert_eq!(replay.rows(), std::slice::from_ref(&relabeled));
        assert!(replay.model(objective_sha256).is_some());
        assert_eq!(
            replay
                .admit_and_refit(relabeled, 12, &OptionValueConfig::default())
                .unwrap_err(),
            HindsightError::DuplicateRelabeledTransition
        );
    }

    #[test]
    fn rejects_already_satisfied_or_detached_transition_evidence() {
        let compiled = compile(&MilestoneProgram {
            version: LanguageVersion { major: 1, minor: 4 },
            definitions: vec![definition(
                "room_one",
                compare(Field::StageRoom, Value::I32(1)),
            )],
        })
        .unwrap();
        let goal = HindsightRelabelDecision::evaluate(&compiled, 0)
            .unwrap()
            .admit()
            .unwrap();
        let transition = option_sample();
        assert_eq!(
            HindsightPredicateEvidence::bind(
                &goal,
                &transition,
                Digest([4; 32]),
                Digest([5; 32]),
                true,
                true,
            ),
            Err(HindsightError::InvalidEvidence)
        );
        let evidence = HindsightPredicateEvidence::bind(
            &goal,
            &transition,
            Digest([4; 32]),
            Digest([5; 32]),
            false,
            true,
        )
        .unwrap();
        let mut detached = transition.clone();
        detached.next_state[0] = 2.0;
        assert_eq!(
            goal.relabel_transition(&detached, evidence, 10.0),
            Err(HindsightError::InvalidEvidence)
        );
    }
}
