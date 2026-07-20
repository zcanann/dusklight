//! Structured model input derived from an authenticated compiled goal graph.
//!
//! Goal identity digests remain provenance only. They are never placed in the
//! feature tensors. Models receive typed predicate nodes, explicit Boolean
//! edges, ordered sequence roots, literal values, selectors, spatial anchors,
//! and projection items. This permits graph/set encoders to share evidence
//! across goals instead of learning arbitrary digest bytes.

use crate::artifact::Digest;
use crate::compiled_goal_graph::{
    CompiledGoalGraph, CompiledGoalGraphError, GoalPredicateNodeKind, GoalPredicateSubject,
};
use crate::milestone_dsl::{
    ActorFact, Comparison, EvaluationPhase, FlagDomain, QueryFact, RngStream, Value,
    ValueProjectionItem,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;

pub const SEMANTIC_GOAL_INPUT_SCHEMA_V1: &str = "dusklight-semantic-goal-input/v1";
pub const GOAL_METADATA_WIDTH: usize = 9;
pub const GOAL_NODE_FEATURE_WIDTH: usize = 78;
pub const GOAL_PROJECTION_FEATURE_WIDTH: usize = 30;
const TOKEN_EMBEDDING_WIDTH: usize = 8;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticGoalInput {
    pub schema: String,
    /// Provenance only; not included in any model feature row.
    pub graph_sha256: Digest,
    pub definition_sha256: Digest,
    pub metadata: Vec<f32>,
    pub node_feature_width: u16,
    pub node_features: Vec<Vec<f32>>,
    pub node_feature_masks: Vec<Vec<f32>>,
    pub edges: Vec<GoalEdgeInput>,
    pub roots: Vec<GoalRootInput>,
    pub projection_feature_width: u16,
    pub projection_features: Vec<Vec<f32>>,
    pub projection_feature_masks: Vec<Vec<f32>>,
    pub input_sha256: Digest,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalEdgeInput {
    pub source: u16,
    pub target: u16,
    pub role: GoalEdgeRole,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalEdgeRole {
    UnaryChild,
    LeftChild,
    RightChild,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalRootInput {
    pub node: u16,
    pub sequence_step: u16,
}

impl SemanticGoalInput {
    pub fn from_graph(graph: &CompiledGoalGraph) -> Result<Self, SemanticGoalInputError> {
        graph.validate().map_err(SemanticGoalInputError::from)?;
        let mut edges = Vec::new();
        let mut node_features = Vec::with_capacity(graph.nodes.len());
        let mut node_feature_masks = Vec::with_capacity(graph.nodes.len());
        for (index, node) in graph.nodes.iter().enumerate() {
            let target = u16::try_from(index).map_err(|_| SemanticGoalInputError::InvalidInput)?;
            match node.kind {
                GoalPredicateNodeKind::Not { child } => edges.push(GoalEdgeInput {
                    source: child,
                    target,
                    role: GoalEdgeRole::UnaryChild,
                }),
                GoalPredicateNodeKind::And { left, right }
                | GoalPredicateNodeKind::Or { left, right } => {
                    edges.push(GoalEdgeInput {
                        source: left,
                        target,
                        role: GoalEdgeRole::LeftChild,
                    });
                    edges.push(GoalEdgeInput {
                        source: right,
                        target,
                        role: GoalEdgeRole::RightChild,
                    });
                }
                GoalPredicateNodeKind::Atom { .. } => {}
            }
            let (features, masks) = encode_node(node, graph.sequence_roots.len());
            node_features.push(features);
            node_feature_masks.push(masks);
        }
        let roots = graph
            .sequence_roots
            .iter()
            .copied()
            .enumerate()
            .map(|(sequence_step, node)| GoalRootInput {
                node,
                sequence_step: sequence_step as u16,
            })
            .collect::<Vec<_>>();
        let mut projection_features = Vec::new();
        let mut projection_feature_masks = Vec::new();
        for (projection_index, projection) in graph.projections.iter().enumerate() {
            for (item_index, item) in projection.items.iter().enumerate() {
                let (features, masks) =
                    encode_projection(projection_index, item_index, &projection.name, item);
                projection_features.push(features);
                projection_feature_masks.push(masks);
            }
        }
        let metadata = vec![
            f32::from(graph.phase == EvaluationPhase::PreInput),
            f32::from(graph.phase == EvaluationPhase::PostSim),
            bounded_log(f64::from(graph.stable_ticks), f64::from(u16::MAX)),
            f32::from(graph.within_ticks.is_some()),
            graph.within_ticks.map_or(0.0, |value| {
                bounded_log(f64::from(value), f64::from(u16::MAX))
            }),
            bounded_log(graph.sequence_roots.len() as f64, f64::from(u16::MAX)),
            bounded_log(graph.nodes.len() as f64, f64::from(u16::MAX)),
            bounded_log(projection_features.len() as f64, f64::from(u16::MAX)),
            bounded_log(
                f64::from(graph.language_version.major) * 256.0
                    + f64::from(graph.language_version.minor),
                f64::from(u16::MAX),
            ),
        ];
        let mut input = Self {
            schema: SEMANTIC_GOAL_INPUT_SCHEMA_V1.into(),
            graph_sha256: graph.graph_sha256,
            definition_sha256: graph.definition_sha256,
            metadata,
            node_feature_width: GOAL_NODE_FEATURE_WIDTH as u16,
            node_features,
            node_feature_masks,
            edges,
            roots,
            projection_feature_width: GOAL_PROJECTION_FEATURE_WIDTH as u16,
            projection_features,
            projection_feature_masks,
            input_sha256: Digest::ZERO,
        };
        input.input_sha256 = input.digest()?;
        input.validate()?;
        Ok(input)
    }

    pub fn validate(&self) -> Result<(), SemanticGoalInputError> {
        let node_count = self.node_features.len();
        if self.schema != SEMANTIC_GOAL_INPUT_SCHEMA_V1
            || self.graph_sha256 == Digest::ZERO
            || self.definition_sha256 == Digest::ZERO
            || self.metadata.len() != GOAL_METADATA_WIDTH
            || self.node_feature_width as usize != GOAL_NODE_FEATURE_WIDTH
            || self.projection_feature_width as usize != GOAL_PROJECTION_FEATURE_WIDTH
            || node_count == 0
            || self.node_feature_masks.len() != node_count
            || self.projection_features.len() != self.projection_feature_masks.len()
            || !valid_rows(
                &self.node_features,
                &self.node_feature_masks,
                GOAL_NODE_FEATURE_WIDTH,
            )
            || !valid_rows(
                &self.projection_features,
                &self.projection_feature_masks,
                GOAL_PROJECTION_FEATURE_WIDTH,
            )
            || self.metadata.iter().any(|value| !value.is_finite())
            || self.roots.is_empty()
            || self.roots.iter().enumerate().any(|(step, root)| {
                usize::from(root.node) >= node_count || root.sequence_step as usize != step
            })
            || self.edges.iter().any(|edge| {
                usize::from(edge.source) >= node_count
                    || usize::from(edge.target) >= node_count
                    || edge.source >= edge.target
            })
            || self.input_sha256 == Digest::ZERO
            || self.input_sha256 != self.digest()?
        {
            return Err(SemanticGoalInputError::InvalidInput);
        }
        Ok(())
    }

    fn digest(&self) -> Result<Digest, SemanticGoalInputError> {
        let bytes = serde_json::to_vec(&(
            &self.schema,
            self.graph_sha256,
            self.definition_sha256,
            &self.metadata,
            self.node_feature_width,
            &self.node_features,
            &self.node_feature_masks,
            &self.edges,
            &self.roots,
            self.projection_feature_width,
            &self.projection_features,
            &self.projection_feature_masks,
        ))
        .map_err(|error| SemanticGoalInputError::Serialization(error.to_string()))?;
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight.semantic-goal-input.identity/v1\0");
        hasher.update(bytes);
        Ok(Digest(hasher.finalize().into()))
    }
}

fn valid_rows(rows: &[Vec<f32>], masks: &[Vec<f32>], width: usize) -> bool {
    rows.iter().zip(masks).all(|(row, mask)| {
        row.len() == width
            && mask.len() == width
            && row.iter().all(|value| value.is_finite())
            && mask.iter().all(|value| *value == 0.0 || *value == 1.0)
    })
}

fn encode_node(
    node: &crate::compiled_goal_graph::GoalPredicateNode,
    sequence_count: usize,
) -> (Vec<f32>, Vec<f32>) {
    let mut output = FeatureRow::default();
    let kind = match node.kind {
        GoalPredicateNodeKind::Atom { .. } => 0,
        GoalPredicateNodeKind::Not { .. } => 1,
        GoalPredicateNodeKind::And { .. } => 2,
        GoalPredicateNodeKind::Or { .. } => 3,
    };
    output.one_hot(kind, 4, true);
    output.values(
        [
            bounded_log(f64::from(node.sequence_step), f64::from(u16::MAX)),
            if sequence_count <= 1 {
                0.0
            } else {
                f32::from(node.sequence_step) / (sequence_count - 1) as f32
            },
        ],
        true,
    );

    let GoalPredicateNodeKind::Atom {
        ref subject,
        operator,
        ref value,
    } = node.kind
    else {
        output.zeros(6 + TOKEN_EMBEDDING_WIDTH + 8 + 8 + 10 + 19 + 6 + 1 + 6);
        debug_assert_eq!(output.values.len(), GOAL_NODE_FEATURE_WIDTH);
        return output.finish();
    };

    let subject_kind = match subject {
        GoalPredicateSubject::Field(_) => 0,
        GoalPredicateSubject::Query(QueryFact::PlacedActor { .. }) => 1,
        GoalPredicateSubject::Query(QueryFact::Flag { .. }) => 2,
        GoalPredicateSubject::Query(QueryFact::TemporaryEventByte { .. }) => 3,
        GoalPredicateSubject::Query(QueryFact::PlayerInAabb { .. }) => 4,
        GoalPredicateSubject::Query(QueryFact::PlayerPlaneSignedDistance { .. }) => 5,
    };
    output.one_hot(subject_kind, 6, true);
    match subject {
        GoalPredicateSubject::Field(field) => {
            output.values(token_embedding(b"goal-field", &[*field as u8]), true)
        }
        GoalPredicateSubject::Query(_) => output.zeros(TOKEN_EMBEDDING_WIDTH),
    }
    output.one_hot(comparison_index(operator), 8, true);
    output.one_hot(value_index(value), 8, true);
    encode_value(value, &mut output);
    encode_query(subject, &mut output);
    debug_assert_eq!(output.values.len(), GOAL_NODE_FEATURE_WIDTH);
    output.finish()
}

fn encode_value(value: &Value, output: &mut FeatureRow) {
    let (bytes, scalar) = match value {
        Value::Bool(value) => (u64::from(*value).to_le_bytes(), f64::from(*value as u8)),
        Value::U32(value) | Value::ProcedureNumber(value) => {
            (u64::from(*value).to_le_bytes(), f64::from(*value))
        }
        Value::U64(value) => (value.to_le_bytes(), *value as f64),
        Value::I32(value) => ((*value as i64 as u64).to_le_bytes(), f64::from(*value)),
        Value::F32(value) => (u64::from(value.to_bits()).to_le_bytes(), f64::from(*value)),
        Value::Symbol(symbol) | Value::ProcedureSymbol(symbol) => {
            output.values(
                token_embedding(b"goal-literal-symbol", symbol.as_bytes()),
                true,
            );
            output.values([0.0, 0.0], false);
            return;
        }
    };
    output.values(bytes.map(|byte| f32::from(byte) / 127.5 - 1.0), true);
    output.values([1.0, signed_log(scalar)], true);
}

fn encode_query(subject: &GoalPredicateSubject, output: &mut FeatureRow) {
    if let GoalPredicateSubject::Query(QueryFact::PlacedActor { selector, field }) = subject {
        output.values(
            token_embedding(b"goal-actor-stage", selector.stage.as_bytes()),
            true,
        );
        output.values(
            [
                f32::from(selector.home_room) / 63.0,
                f32::from(selector.set_id) / f32::from(u16::MAX),
                f32::from(selector.actor_name) / f32::from(i16::MAX),
            ],
            true,
        );
        output.one_hot(actor_fact_index(field), 8, true);
    } else {
        output.zeros(19);
    }
    if let GoalPredicateSubject::Query(QueryFact::Flag { selector }) = subject {
        output.one_hot(flag_domain_index(selector.domain), 4, true);
        output.values(
            [
                f32::from(selector.room) / 63.0,
                f32::from(selector.index) / f32::from(u16::MAX),
            ],
            true,
        );
    } else {
        output.zeros(6);
    }
    if let GoalPredicateSubject::Query(QueryFact::TemporaryEventByte { index }) = subject {
        output.values([f32::from(*index) / f32::from(u16::MAX)], true);
    } else {
        output.zeros(1);
    }
    match subject {
        GoalPredicateSubject::Query(QueryFact::PlayerInAabb { minimum, maximum }) => output.values(
            minimum
                .iter()
                .chain(maximum)
                .map(|value| signed_log(f64::from(*value))),
            true,
        ),
        GoalPredicateSubject::Query(QueryFact::PlayerPlaneSignedDistance { point, normal }) => {
            output.values(
                point
                    .iter()
                    .chain(normal)
                    .map(|value| signed_log(f64::from(*value))),
                true,
            );
        }
        _ => output.zeros(6),
    }
}

fn encode_projection(
    projection_index: usize,
    item_index: usize,
    name: &str,
    item: &ValueProjectionItem,
) -> (Vec<f32>, Vec<f32>) {
    let mut output = FeatureRow::default();
    let kind = match item {
        ValueProjectionItem::Rng { .. } => 0,
        ValueProjectionItem::ActorPopulation { .. } => 1,
        ValueProjectionItem::Flag { .. } => 2,
    };
    output.one_hot(kind, 3, true);
    output.values(
        [
            bounded_log(projection_index as f64, f64::from(u16::MAX)),
            bounded_log(item_index as f64, f64::from(u16::MAX)),
        ],
        true,
    );
    output.values(
        token_embedding(b"goal-projection-name", name.as_bytes()),
        true,
    );
    match item {
        ValueProjectionItem::Rng { stream } => {
            output.one_hot(rng_stream_index(*stream), 2, true);
            output.zeros(TOKEN_EMBEDDING_WIDTH + 1 + 4 + 2);
        }
        ValueProjectionItem::ActorPopulation { stage, room } => {
            output.zeros(2);
            output.values(
                token_embedding(b"goal-projection-stage", stage.as_bytes()),
                true,
            );
            output.values([f32::from(*room) / 63.0], true);
            output.zeros(4 + 2);
        }
        ValueProjectionItem::Flag { selector } => {
            output.zeros(2 + TOKEN_EMBEDDING_WIDTH + 1);
            output.one_hot(flag_domain_index(selector.domain), 4, true);
            output.values(
                [
                    f32::from(selector.room) / 63.0,
                    f32::from(selector.index) / f32::from(u16::MAX),
                ],
                true,
            );
        }
    }
    debug_assert_eq!(output.values.len(), GOAL_PROJECTION_FEATURE_WIDTH);
    output.finish()
}

#[derive(Default)]
struct FeatureRow {
    values: Vec<f32>,
    masks: Vec<f32>,
}

impl FeatureRow {
    fn values(&mut self, values: impl IntoIterator<Item = f32>, present: bool) {
        let values = values.into_iter().collect::<Vec<_>>();
        self.masks
            .extend(std::iter::repeat_n(f32::from(present), values.len()));
        self.values.extend(values);
    }

    fn zeros(&mut self, count: usize) {
        self.values.extend(std::iter::repeat_n(0.0, count));
        self.masks.extend(std::iter::repeat_n(0.0, count));
    }

    fn one_hot(&mut self, index: usize, width: usize, present: bool) {
        self.values(
            (0..width).map(|candidate| f32::from(candidate == index)),
            present,
        );
    }

    fn finish(self) -> (Vec<f32>, Vec<f32>) {
        (self.values, self.masks)
    }
}

fn token_embedding(domain: &[u8], token: &[u8]) -> [f32; TOKEN_EMBEDDING_WIDTH] {
    std::array::from_fn(|dimension| {
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight.semantic-goal-token/v1\0");
        hasher.update((domain.len() as u64).to_le_bytes());
        hasher.update(domain);
        hasher.update((token.len() as u64).to_le_bytes());
        hasher.update(token);
        hasher.update((dimension as u64).to_le_bytes());
        let digest = hasher.finalize();
        let raw = u32::from_le_bytes(digest[..4].try_into().expect("four bytes"));
        (raw as f32 / u32::MAX as f32) * 2.0 - 1.0
    })
}

fn signed_log(value: f64) -> f32 {
    (value.signum() * value.abs().ln_1p()).clamp(-64.0, 64.0) as f32 / 64.0
}

fn bounded_log(value: f64, maximum: f64) -> f32 {
    (value.max(0.0).ln_1p() / maximum.ln_1p()) as f32
}

const fn comparison_index(value: Comparison) -> usize {
    value as u8 as usize - Comparison::Equal as u8 as usize
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

const fn actor_fact_index(value: &ActorFact) -> usize {
    *value as u8 as usize - 1
}

const fn flag_domain_index(value: FlagDomain) -> usize {
    value as u8 as usize
}

const fn rng_stream_index(value: RngStream) -> usize {
    value as u8 as usize
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SemanticGoalInputError {
    InvalidGraph(String),
    InvalidInput,
    Serialization(String),
}

impl fmt::Display for SemanticGoalInputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidGraph(message) => write!(formatter, "compiled goal is invalid: {message}"),
            Self::InvalidInput => formatter.write_str("semantic goal model input is invalid"),
            Self::Serialization(message) => {
                write!(
                    formatter,
                    "semantic goal model input serialization failed: {message}"
                )
            }
        }
    }
}

impl Error for SemanticGoalInputError {}

impl From<CompiledGoalGraphError> for SemanticGoalInputError {
    fn from(error: CompiledGoalGraphError) -> Self {
        Self::InvalidGraph(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiled_goal_graph::CompiledGoalGraph;
    use crate::milestone_dsl::compile_source;

    const SOURCE: &str = r#"milestones 1.8
milestone semantic_goal {
  phase post_sim
  when stage.room == 1 && player.in_aabb(-10.0, 0.0, 20.0, 10.0, 40.0, 60.0)
  then actor.placed.distance_to_player("F_SP104", 1, 12, 7) < 35.0
  within 90
  projection parity {
    rng primary
    actor_population "F_SP104" 1
  }
}
"#;

    fn input(source: &str) -> SemanticGoalInput {
        let compiled = compile_source(source).unwrap();
        let graph = CompiledGoalGraph::from_compiled(&compiled, 0).unwrap();
        SemanticGoalInput::from_graph(&graph).unwrap()
    }

    #[test]
    fn model_input_contains_typed_nodes_edges_roots_and_projections() {
        let input = input(SOURCE);
        assert_eq!(input.metadata.len(), GOAL_METADATA_WIDTH);
        assert_eq!(input.node_features.len(), 4);
        assert_eq!(input.edges.len(), 2);
        assert_eq!(input.roots.len(), 2);
        assert_eq!(input.projection_features.len(), 2);
        assert!(
            input
                .node_features
                .iter()
                .all(|row| row.len() == GOAL_NODE_FEATURE_WIDTH)
        );
        assert_ne!(input.input_sha256, Digest::ZERO);
        input.validate().unwrap();
    }

    #[test]
    fn literal_and_topology_changes_change_model_features_not_only_identity() {
        let original = input(SOURCE);
        let literal = input(&SOURCE.replace("stage.room == 1", "stage.room == 2"));
        assert_ne!(original.node_features, literal.node_features);

        let topology = input(&SOURCE.replace("stage.room == 1 &&", "stage.room == 1 ||"));
        assert_eq!(original.node_features.len(), topology.node_features.len());
        assert_ne!(original.node_features, topology.node_features);
        assert_eq!(original.edges, topology.edges);
    }

    #[test]
    fn identity_digest_is_provenance_not_a_feature() {
        let original = input(SOURCE);
        let renamed = input(&SOURCE.replace("semantic_goal", "renamed_goal"));
        assert_ne!(original.definition_sha256, renamed.definition_sha256);
        assert_eq!(original.metadata, renamed.metadata);
        assert_eq!(original.node_features, renamed.node_features);
        assert_eq!(original.edges, renamed.edges);
        assert_eq!(original.roots, renamed.roots);
        assert_eq!(original.projection_features, renamed.projection_features);
    }

    #[test]
    fn serialized_tampering_fails_closed() {
        let input = input(SOURCE);
        let mut decoded: SemanticGoalInput =
            serde_json::from_slice(&serde_json::to_vec(&input).unwrap()).unwrap();
        decoded.validate().unwrap();
        decoded.node_features[0][0] = 0.25;
        assert_eq!(
            decoded.validate(),
            Err(SemanticGoalInputError::InvalidInput)
        );
    }
}
