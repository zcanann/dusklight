//! Authenticated, model-facing structure for one compiled milestone goal.
//!
//! This is deliberately not a bag of predicate counts or opaque digest bytes.
//! Every Boolean edge, typed subject, comparison, literal, and sequence step is
//! retained so set/graph encoders can learn across goals without source text.

use crate::artifact::Digest;
use crate::milestone_dsl::{
    Comparison, CompiledMilestones, EvaluationPhase, Expression, Field, LanguageVersion,
    MilestoneDefinition, MilestoneProgram, QueryFact, Value, ValueProjection, compile, decode,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;

pub const COMPILED_GOAL_GRAPH_SCHEMA_V1: &str = "dusklight-compiled-goal-graph/v1";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledGoalGraph {
    pub schema: String,
    pub program_sha256: Digest,
    pub definition_sha256: Digest,
    pub definition_name: String,
    pub language_version: LanguageVersion,
    pub phase: EvaluationPhase,
    pub stable_ticks: u16,
    pub within_ticks: Option<u16>,
    /// One root for `when`, followed by each ordered `then` predicate.
    pub sequence_roots: Vec<u16>,
    /// Canonical postorder nodes. Child indices therefore precede parents.
    pub nodes: Vec<GoalPredicateNode>,
    pub projections: Vec<ValueProjection>,
    pub graph_sha256: Digest,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalPredicateNode {
    pub sequence_step: u16,
    pub kind: GoalPredicateNodeKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GoalPredicateNodeKind {
    Atom {
        subject: GoalPredicateSubject,
        operator: Comparison,
        value: Value,
    },
    Not {
        child: u16,
    },
    And {
        left: u16,
        right: u16,
    },
    Or {
        left: u16,
        right: u16,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum GoalPredicateSubject {
    Field(Field),
    Query(QueryFact),
}

#[derive(Clone, Debug, PartialEq)]
pub enum GoalSpatialAnchor {
    PlacedActor {
        node_index: u16,
        sequence_step: u16,
        selector: crate::milestone_dsl::PlacedActorSelector,
    },
    PlayerAabb {
        node_index: u16,
        sequence_step: u16,
        minimum: [f32; 3],
        maximum: [f32; 3],
    },
    PlayerPlane {
        node_index: u16,
        sequence_step: u16,
        point: [f32; 3],
        normal: [f32; 3],
    },
}

impl CompiledGoalGraph {
    pub fn from_compiled(
        compiled: &CompiledMilestones,
        definition_index: usize,
    ) -> Result<Self, CompiledGoalGraphError> {
        let decoded = decode(&compiled.bytes)
            .map_err(|error| CompiledGoalGraphError::InvalidProgram(error.to_string()))?;
        if decoded.program_sha256 != compiled.program_sha256
            || decoded.definitions != compiled.definitions
        {
            return Err(CompiledGoalGraphError::InvalidProgram(
                "compiled milestone identity differs from canonical bytes".into(),
            ));
        }
        let definition = decoded
            .program
            .definitions
            .get(definition_index)
            .ok_or(CompiledGoalGraphError::UnknownDefinition(definition_index))?;
        let identity = decoded
            .definitions
            .get(definition_index)
            .ok_or(CompiledGoalGraphError::UnknownDefinition(definition_index))?;

        let mut nodes = Vec::new();
        let mut sequence_roots = Vec::with_capacity(1 + definition.then.len());
        for (step, expression) in std::iter::once(&definition.when)
            .chain(&definition.then)
            .enumerate()
        {
            sequence_roots.push(push_expression(expression, step as u16, &mut nodes)?);
        }

        let mut graph = Self {
            schema: COMPILED_GOAL_GRAPH_SCHEMA_V1.into(),
            program_sha256: Digest(decoded.program_sha256),
            definition_sha256: Digest(identity.sha256),
            definition_name: identity.name.clone(),
            language_version: decoded.program.version,
            phase: definition.phase,
            stable_ticks: definition.stable_ticks,
            within_ticks: definition.within_ticks,
            sequence_roots,
            nodes,
            projections: definition.projections.clone(),
            graph_sha256: Digest::ZERO,
        };
        let reconstructed = graph.reconstruct_definition()?;
        graph.graph_sha256 =
            graph_digest(graph.program_sha256, graph.language_version, &reconstructed)?;
        graph.validate()?;
        Ok(graph)
    }

    pub fn validate(&self) -> Result<(), CompiledGoalGraphError> {
        if self.schema != COMPILED_GOAL_GRAPH_SCHEMA_V1
            || self.program_sha256 == Digest::ZERO
            || self.definition_sha256 == Digest::ZERO
            || self.graph_sha256 == Digest::ZERO
            || self.definition_name.is_empty()
        {
            return Err(CompiledGoalGraphError::InvalidGraph(
                "identity metadata is incomplete".into(),
            ));
        }
        let definition = self.reconstruct_definition()?;
        let compiled = compile(&MilestoneProgram {
            version: self.language_version,
            definitions: vec![definition.clone()],
        })
        .map_err(|error| CompiledGoalGraphError::InvalidGraph(error.to_string()))?;
        if compiled.definitions.len() != 1
            || Digest(compiled.definitions[0].sha256) != self.definition_sha256
            || compiled.definitions[0].name != self.definition_name
            || graph_digest(self.program_sha256, self.language_version, &definition)?
                != self.graph_sha256
        {
            return Err(CompiledGoalGraphError::InvalidGraph(
                "graph does not reproduce its compiled definition identity".into(),
            ));
        }
        Ok(())
    }

    pub fn spatial_anchors(&self) -> Vec<GoalSpatialAnchor> {
        self.nodes
            .iter()
            .enumerate()
            .filter_map(|(index, node)| {
                let GoalPredicateNodeKind::Atom {
                    subject: GoalPredicateSubject::Query(query),
                    ..
                } = &node.kind
                else {
                    return None;
                };
                let node_index = index as u16;
                match query {
                    QueryFact::PlacedActor { selector, .. } => {
                        Some(GoalSpatialAnchor::PlacedActor {
                            node_index,
                            sequence_step: node.sequence_step,
                            selector: selector.clone(),
                        })
                    }
                    QueryFact::PlayerInAabb { minimum, maximum } => {
                        Some(GoalSpatialAnchor::PlayerAabb {
                            node_index,
                            sequence_step: node.sequence_step,
                            minimum: *minimum,
                            maximum: *maximum,
                        })
                    }
                    QueryFact::PlayerPlaneSignedDistance { point, normal } => {
                        Some(GoalSpatialAnchor::PlayerPlane {
                            node_index,
                            sequence_step: node.sequence_step,
                            point: *point,
                            normal: *normal,
                        })
                    }
                    QueryFact::Flag { .. } | QueryFact::TemporaryEventByte { .. } => None,
                }
            })
            .collect()
    }

    fn reconstruct_definition(&self) -> Result<MilestoneDefinition, CompiledGoalGraphError> {
        if self.sequence_roots.is_empty() || self.nodes.is_empty() {
            return Err(CompiledGoalGraphError::InvalidGraph(
                "predicate graph has no root or node".into(),
            ));
        }
        let mut marks = vec![VisitMark::Unseen; self.nodes.len()];
        let mut expressions = Vec::with_capacity(self.sequence_roots.len());
        for (step, root) in self.sequence_roots.iter().copied().enumerate() {
            expressions.push(reconstruct_expression(
                usize::from(root),
                step as u16,
                &self.nodes,
                &mut marks,
            )?);
        }
        if marks.iter().any(|mark| *mark != VisitMark::Done) {
            return Err(CompiledGoalGraphError::InvalidGraph(
                "predicate graph contains unreachable nodes".into(),
            ));
        }
        let when = expressions.remove(0);
        Ok(MilestoneDefinition {
            name: self.definition_name.clone(),
            phase: self.phase,
            stable_ticks: self.stable_ticks,
            when,
            then: expressions,
            within_ticks: self.within_ticks,
            projections: self.projections.clone(),
        })
    }
}

fn push_expression(
    expression: &Expression,
    sequence_step: u16,
    nodes: &mut Vec<GoalPredicateNode>,
) -> Result<u16, CompiledGoalGraphError> {
    let kind = match expression {
        Expression::Compare {
            field,
            operator,
            value,
        } => GoalPredicateNodeKind::Atom {
            subject: GoalPredicateSubject::Field(*field),
            operator: *operator,
            value: value.clone(),
        },
        Expression::Query {
            fact,
            operator,
            value,
        } => GoalPredicateNodeKind::Atom {
            subject: GoalPredicateSubject::Query(fact.clone()),
            operator: *operator,
            value: value.clone(),
        },
        Expression::Not(child) => GoalPredicateNodeKind::Not {
            child: push_expression(child, sequence_step, nodes)?,
        },
        Expression::And(left, right) => GoalPredicateNodeKind::And {
            left: push_expression(left, sequence_step, nodes)?,
            right: push_expression(right, sequence_step, nodes)?,
        },
        Expression::Or(left, right) => GoalPredicateNodeKind::Or {
            left: push_expression(left, sequence_step, nodes)?,
            right: push_expression(right, sequence_step, nodes)?,
        },
    };
    let index = u16::try_from(nodes.len()).map_err(|_| {
        CompiledGoalGraphError::InvalidGraph("predicate graph exceeds u16 indices".into())
    })?;
    nodes.push(GoalPredicateNode {
        sequence_step,
        kind,
    });
    Ok(index)
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum VisitMark {
    Unseen,
    Active,
    Done,
}

fn reconstruct_expression(
    index: usize,
    sequence_step: u16,
    nodes: &[GoalPredicateNode],
    marks: &mut [VisitMark],
) -> Result<Expression, CompiledGoalGraphError> {
    let node = nodes.get(index).ok_or_else(|| {
        CompiledGoalGraphError::InvalidGraph("predicate child index is out of range".into())
    })?;
    if node.sequence_step != sequence_step || marks[index] != VisitMark::Unseen {
        return Err(CompiledGoalGraphError::InvalidGraph(
            "predicate graph has a cycle, shared node, or crossed sequence step".into(),
        ));
    }
    marks[index] = VisitMark::Active;
    let expression =
        match &node.kind {
            GoalPredicateNodeKind::Atom {
                subject: GoalPredicateSubject::Field(field),
                operator,
                value,
            } => Expression::Compare {
                field: *field,
                operator: *operator,
                value: value.clone(),
            },
            GoalPredicateNodeKind::Atom {
                subject: GoalPredicateSubject::Query(fact),
                operator,
                value,
            } => Expression::Query {
                fact: fact.clone(),
                operator: *operator,
                value: value.clone(),
            },
            GoalPredicateNodeKind::Not { child } => Expression::Not(Box::new(
                reconstruct_expression(usize::from(*child), sequence_step, nodes, marks)?,
            )),
            GoalPredicateNodeKind::And { left, right } => Expression::And(
                Box::new(reconstruct_expression(
                    usize::from(*left),
                    sequence_step,
                    nodes,
                    marks,
                )?),
                Box::new(reconstruct_expression(
                    usize::from(*right),
                    sequence_step,
                    nodes,
                    marks,
                )?),
            ),
            GoalPredicateNodeKind::Or { left, right } => Expression::Or(
                Box::new(reconstruct_expression(
                    usize::from(*left),
                    sequence_step,
                    nodes,
                    marks,
                )?),
                Box::new(reconstruct_expression(
                    usize::from(*right),
                    sequence_step,
                    nodes,
                    marks,
                )?),
            ),
        };
    marks[index] = VisitMark::Done;
    Ok(expression)
}

fn graph_digest(
    program_sha256: Digest,
    language_version: LanguageVersion,
    definition: &MilestoneDefinition,
) -> Result<Digest, CompiledGoalGraphError> {
    let single = compile(&MilestoneProgram {
        version: language_version,
        definitions: vec![definition.clone()],
    })
    .map_err(|error| CompiledGoalGraphError::InvalidGraph(error.to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight.compiled-goal-graph.identity/v1\0");
    hasher.update(program_sha256.0);
    hasher.update((single.bytes.len() as u64).to_le_bytes());
    hasher.update(single.bytes);
    Ok(Digest(hasher.finalize().into()))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CompiledGoalGraphError {
    InvalidProgram(String),
    UnknownDefinition(usize),
    InvalidGraph(String),
}

impl fmt::Display for CompiledGoalGraphError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidProgram(message) => {
                write!(formatter, "compiled goal is invalid: {message}")
            }
            Self::UnknownDefinition(index) => {
                write!(formatter, "compiled goal definition {index} does not exist")
            }
            Self::InvalidGraph(message) => {
                write!(formatter, "compiled goal graph is invalid: {message}")
            }
        }
    }
}

impl Error for CompiledGoalGraphError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::milestone_dsl::compile_source;

    const SOURCE: &str = r#"milestones 1.8
milestone exact_goal {
  phase post_sim
  when stage.room == 1 && player.in_aabb(-10.0, 0.0, 20.0, 10.0, 40.0, 60.0)
  then actor.placed.distance_to_player("F_SP104", 1, 12, 7) < 35.0
  within 90
}
"#;
    const ORDON_SOURCE: &str = include_str!(
        "../../../../../routes/Glitch Exhibition/intro/predicates/ordon_spring_load_committed.milestones"
    );

    #[test]
    fn graph_retains_typed_operands_topology_and_sequence() {
        let compiled = compile_source(SOURCE).unwrap();
        let graph = CompiledGoalGraph::from_compiled(&compiled, 0).unwrap();
        assert_eq!(graph.sequence_roots.len(), 2);
        assert_eq!(graph.nodes.len(), 4);
        assert_eq!(graph.phase, EvaluationPhase::PostSim);
        assert_eq!(graph.stable_ticks, 1);
        assert_eq!(graph.within_ticks, Some(90));
        assert_eq!(graph.spatial_anchors().len(), 2);
        assert_ne!(graph.graph_sha256, Digest::ZERO);
        graph.validate().unwrap();
        let decoded: CompiledGoalGraph =
            serde_json::from_slice(&serde_json::to_vec(&graph).unwrap()).unwrap();
        assert_eq!(decoded, graph);
        decoded.validate().unwrap();

        assert!(graph.nodes.iter().any(|node| matches!(
            node.kind,
            GoalPredicateNodeKind::Atom {
                subject: GoalPredicateSubject::Field(Field::StageRoom),
                operator: Comparison::Equal,
                value: Value::I32(1),
            }
        )));
    }

    #[test]
    fn literal_changes_are_semantic_not_only_opaque_hash_changes() {
        let first = CompiledGoalGraph::from_compiled(&compile_source(SOURCE).unwrap(), 0).unwrap();
        let changed_source = SOURCE.replace("stage.room == 1", "stage.room == 2");
        let second =
            CompiledGoalGraph::from_compiled(&compile_source(&changed_source).unwrap(), 0).unwrap();
        let literal = |graph: &CompiledGoalGraph| {
            graph.nodes.iter().find_map(|node| match node.kind {
                GoalPredicateNodeKind::Atom {
                    subject: GoalPredicateSubject::Field(Field::StageRoom),
                    value: Value::I32(value),
                    ..
                } => Some(value),
                _ => None,
            })
        };
        assert_eq!(literal(&first), Some(1));
        assert_eq!(literal(&second), Some(2));
        assert_ne!(first.definition_sha256, second.definition_sha256);
        assert_ne!(first.graph_sha256, second.graph_sha256);
    }

    #[test]
    fn topology_tampering_fails_closed() {
        let compiled = compile_source(SOURCE).unwrap();
        let mut graph = CompiledGoalGraph::from_compiled(&compiled, 0).unwrap();
        let root = usize::from(graph.sequence_roots[0]);
        graph.nodes[root].kind = GoalPredicateNodeKind::Not {
            child: graph.sequence_roots[0],
        };
        assert!(graph.validate().is_err());
    }

    #[test]
    fn shipped_ordon_goal_exposes_terminal_semantics_without_fake_spatial_anchor() {
        let compiled = compile_source(ORDON_SOURCE).unwrap();
        let graph = CompiledGoalGraph::from_compiled(&compiled, 0).unwrap();
        assert_eq!(
            graph.definition_sha256.to_string(),
            "631b025f41e16251e47f340fb0030fab07be15433204d2fdef8eb08915b11e57"
        );
        assert_eq!(graph.nodes.len(), 9);
        assert!(graph.spatial_anchors().is_empty());
        assert!(graph.nodes.iter().any(|node| matches!(
            &node.kind,
            GoalPredicateNodeKind::Atom {
                subject: GoalPredicateSubject::Field(Field::NextStageName),
                operator: Comparison::Equal,
                value: Value::Symbol(value),
            } if value == "F_SP104"
        )));
        graph.validate().unwrap();
    }
}
