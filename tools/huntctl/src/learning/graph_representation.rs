//! Equal-row comparisons for bounded actor and local-collision graph encoders.

use crate::artifact::Digest;
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

pub const GRAPH_ENCODER_COMPARISON_SCHEMA_V1: &str = "dusklight-graph-encoder-comparison/v1";
const MAX_SAMPLES: usize = 100_000;
const MAX_NODES: usize = 128;
const MAX_EDGES: usize = 1024;
const MAX_FEATURE_WIDTH: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphDomain {
    ActorRelationships,
    LocalCollision,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct GraphEdge {
    pub source: usize,
    pub target: usize,
    pub weight: f32,
}

#[derive(Clone, Debug)]
pub struct GraphSample {
    pub sample_sha256: Digest,
    pub base_features: Vec<f32>,
    pub nodes: Vec<Vec<f32>>,
    pub edges: Vec<GraphEdge>,
    pub target: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct GraphComparisonConfig {
    pub minimum_training_samples: usize,
    pub minimum_held_out_samples: usize,
    pub ridge_penalty: f64,
    pub minimum_relative_improvement: f64,
}

impl Default for GraphComparisonConfig {
    fn default() -> Self {
        Self {
            minimum_training_samples: 2048,
            minimum_held_out_samples: 512,
            ridge_penalty: 1.0e-3,
            minimum_relative_improvement: 0.1,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphEncoderDecision {
    RetainSimplerRepresentation,
    GraphEncoderCandidate,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GraphCriticMetrics {
    pub name: &'static str,
    pub feature_width: usize,
    pub training_rows: usize,
    pub held_out_rows: usize,
    pub training_mse: f64,
    pub held_out_mse: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GraphEncoderComparison {
    pub schema: &'static str,
    pub domain: GraphDomain,
    pub representation_sha256: Digest,
    pub simpler_baseline_report_sha256: Digest,
    pub training_dataset_sha256: Digest,
    pub held_out_dataset_sha256: Digest,
    pub config: GraphComparisonConfig,
    pub node_feature_width: usize,
    pub edge_semantics: &'static str,
    pub simpler: GraphCriticMetrics,
    pub message_passing: GraphCriticMetrics,
    pub equal_training_row_budget: bool,
    pub equal_held_out_row_budget: bool,
    pub relative_held_out_improvement: f64,
    pub decision: GraphEncoderDecision,
    pub promotion_authority: bool,
    pub comparison_sha256: Digest,
}

impl GraphEncoderComparison {
    #[allow(clippy::too_many_arguments)]
    pub fn evaluate(
        domain: GraphDomain,
        representation_sha256: Digest,
        simpler_baseline_report_sha256: Digest,
        training_dataset_sha256: Digest,
        held_out_dataset_sha256: Digest,
        training: &[GraphSample],
        held_out: &[GraphSample],
        config: GraphComparisonConfig,
    ) -> Result<Self, GraphRepresentationError> {
        let node_feature_width = validate_inputs(
            representation_sha256,
            simpler_baseline_report_sha256,
            training_dataset_sha256,
            held_out_dataset_sha256,
            training,
            held_out,
            config,
        )?;
        let train_simple = rows(training, false, node_feature_width);
        let held_out_simple = rows(held_out, false, node_feature_width);
        let train_graph = rows(training, true, node_feature_width);
        let held_out_graph = rows(held_out, true, node_feature_width);
        let simpler = fit_evaluate(
            "pooled_fixed_baseline",
            &train_simple,
            &held_out_simple,
            config,
        )?;
        let message_passing = fit_evaluate(
            "one_hop_message_passing",
            &train_graph,
            &held_out_graph,
            config,
        )?;
        let equal_training_row_budget = simpler.training_rows == message_passing.training_rows;
        let equal_held_out_row_budget = simpler.held_out_rows == message_passing.held_out_rows;
        if !equal_training_row_budget || !equal_held_out_row_budget {
            return Err(GraphRepresentationError::new(
                "graph comparison did not preserve equal row budgets",
            ));
        }
        let relative_held_out_improvement = if simpler.held_out_mse > f64::EPSILON {
            (simpler.held_out_mse - message_passing.held_out_mse) / simpler.held_out_mse
        } else {
            0.0
        };
        let decision = if relative_held_out_improvement >= config.minimum_relative_improvement {
            GraphEncoderDecision::GraphEncoderCandidate
        } else {
            GraphEncoderDecision::RetainSimplerRepresentation
        };
        let mut report = Self {
            schema: GRAPH_ENCODER_COMPARISON_SCHEMA_V1,
            domain,
            representation_sha256,
            simpler_baseline_report_sha256,
            training_dataset_sha256,
            held_out_dataset_sha256,
            config,
            node_feature_width,
            edge_semantics: "directed_weighted_one_hop",
            simpler,
            message_passing,
            equal_training_row_budget,
            equal_held_out_row_budget,
            relative_held_out_improvement,
            decision,
            promotion_authority: false,
            comparison_sha256: Digest::ZERO,
        };
        report.comparison_sha256 = report.digest()?;
        Ok(report)
    }

    fn digest(&self) -> Result<Digest, GraphRepresentationError> {
        canonical_digest(
            b"dusklight.graph-encoder-comparison/v1\0",
            &(
                self.schema,
                self.domain,
                self.representation_sha256,
                self.simpler_baseline_report_sha256,
                self.training_dataset_sha256,
                self.held_out_dataset_sha256,
                self.config,
                self.node_feature_width,
                self.edge_semantics,
                &self.simpler,
                &self.message_passing,
                self.equal_training_row_budget,
                self.equal_held_out_row_budget,
                self.relative_held_out_improvement,
                self.decision,
                self.promotion_authority,
            ),
        )
    }
}

#[derive(Clone)]
struct RegressionRows {
    features: Vec<Vec<f64>>,
    targets: Vec<f64>,
}

fn rows(samples: &[GraphSample], graph: bool, node_width: usize) -> RegressionRows {
    RegressionRows {
        features: samples
            .iter()
            .map(|sample| encode(sample, graph, node_width))
            .collect(),
        targets: samples
            .iter()
            .map(|sample| f64::from(sample.target))
            .collect(),
    }
}

fn encode(sample: &GraphSample, graph: bool, width: usize) -> Vec<f64> {
    let mut output = vec![1.0];
    output.extend(sample.base_features.iter().map(|value| f64::from(*value)));
    output.push(sample.nodes.len() as f64 / MAX_NODES as f64);
    output.push(sample.edges.len() as f64 / MAX_EDGES as f64);
    let mut mean = vec![0.0_f64; width];
    let mut minimum = vec![f64::INFINITY; width];
    let mut maximum = vec![f64::NEG_INFINITY; width];
    for node in &sample.nodes {
        for index in 0..width {
            let value = f64::from(node[index]);
            mean[index] += value;
            minimum[index] = minimum[index].min(value);
            maximum[index] = maximum[index].max(value);
        }
    }
    for value in &mut mean {
        *value /= sample.nodes.len() as f64;
    }
    output.extend(mean);
    output.extend(minimum);
    output.extend(maximum);
    if !graph {
        return output;
    }
    let mut messages = vec![vec![0.0_f64; width]; sample.nodes.len()];
    let mut degrees = vec![0_u32; sample.nodes.len()];
    for edge in &sample.edges {
        degrees[edge.target] += 1;
        for index in 0..width {
            messages[edge.target][index] +=
                f64::from(sample.nodes[edge.source][index]) * f64::from(edge.weight);
        }
    }
    let mut message_mean = vec![0.0; width];
    let mut message_maximum = vec![f64::NEG_INFINITY; width];
    for message in &messages {
        for index in 0..width {
            message_mean[index] += message[index];
            message_maximum[index] = message_maximum[index].max(message[index]);
        }
    }
    for value in &mut message_mean {
        *value /= messages.len() as f64;
    }
    output.extend(message_mean);
    output.extend(message_maximum);
    output.push(degrees.iter().copied().max().unwrap_or(0) as f64 / MAX_NODES as f64);
    output
}

fn fit_evaluate(
    name: &'static str,
    training: &RegressionRows,
    held_out: &RegressionRows,
    config: GraphComparisonConfig,
) -> Result<GraphCriticMetrics, GraphRepresentationError> {
    let weights = ridge_fit(training, config.ridge_penalty)?;
    Ok(GraphCriticMetrics {
        name,
        feature_width: weights.len(),
        training_rows: training.features.len(),
        held_out_rows: held_out.features.len(),
        training_mse: mse(training, &weights),
        held_out_mse: mse(held_out, &weights),
    })
}

fn ridge_fit(rows: &RegressionRows, penalty: f64) -> Result<Vec<f64>, GraphRepresentationError> {
    let width = rows.features[0].len();
    let mut matrix = vec![vec![0.0; width + 1]; width];
    for (features, target) in rows.features.iter().zip(&rows.targets) {
        for row in 0..width {
            matrix[row][width] += features[row] * target;
            for column in 0..width {
                matrix[row][column] += features[row] * features[column];
            }
        }
    }
    for (index, row) in matrix.iter_mut().enumerate().skip(1) {
        row[index] += penalty;
    }
    for pivot in 0..width {
        let best = (pivot..width)
            .max_by(|left, right| {
                matrix[*left][pivot]
                    .abs()
                    .total_cmp(&matrix[*right][pivot].abs())
            })
            .unwrap();
        if matrix[best][pivot].abs() <= 1.0e-12 {
            return Err(GraphRepresentationError::new(
                "graph comparison ridge system is singular",
            ));
        }
        matrix.swap(pivot, best);
        let divisor = matrix[pivot][pivot];
        for column in pivot..=width {
            matrix[pivot][column] /= divisor;
        }
        for row in 0..width {
            if row == pivot {
                continue;
            }
            let factor = matrix[row][pivot];
            for column in pivot..=width {
                matrix[row][column] -= factor * matrix[pivot][column];
            }
        }
    }
    Ok(matrix.into_iter().map(|row| row[width]).collect())
}

fn mse(rows: &RegressionRows, weights: &[f64]) -> f64 {
    rows.features
        .iter()
        .zip(&rows.targets)
        .map(|(features, target)| {
            let prediction = features
                .iter()
                .zip(weights)
                .map(|(feature, weight)| feature * weight)
                .sum::<f64>();
            (prediction - target).powi(2)
        })
        .sum::<f64>()
        / rows.features.len() as f64
}

fn validate_inputs(
    representation_sha256: Digest,
    simpler_baseline_report_sha256: Digest,
    training_dataset_sha256: Digest,
    held_out_dataset_sha256: Digest,
    training: &[GraphSample],
    held_out: &[GraphSample],
    config: GraphComparisonConfig,
) -> Result<usize, GraphRepresentationError> {
    if representation_sha256 == Digest::ZERO
        || simpler_baseline_report_sha256 == Digest::ZERO
        || training_dataset_sha256 == Digest::ZERO
        || held_out_dataset_sha256 == Digest::ZERO
        || training_dataset_sha256 == held_out_dataset_sha256
        || training.len() < config.minimum_training_samples
        || held_out.len() < config.minimum_held_out_samples
        || training.len() > MAX_SAMPLES
        || held_out.len() > MAX_SAMPLES
        || config.minimum_training_samples == 0
        || config.minimum_held_out_samples == 0
        || !config.ridge_penalty.is_finite()
        || config.ridge_penalty <= 0.0
        || !config.minimum_relative_improvement.is_finite()
        || !(0.0..=1.0).contains(&config.minimum_relative_improvement)
    {
        return Err(GraphRepresentationError::new(
            "graph comparison configuration or dataset identity is invalid",
        ));
    }
    let first = &training[0];
    let node_width = first.nodes.first().map(Vec::len).unwrap_or(0);
    let base_width = first.base_features.len();
    if node_width == 0 || node_width > MAX_FEATURE_WIDTH || base_width > MAX_FEATURE_WIDTH {
        return Err(GraphRepresentationError::new(
            "graph sample feature width is invalid",
        ));
    }
    let mut identities = BTreeSet::new();
    for sample in training.iter().chain(held_out) {
        if sample.sample_sha256 == Digest::ZERO
            || !identities.insert(sample.sample_sha256)
            || sample.base_features.len() != base_width
            || sample.nodes.is_empty()
            || sample.nodes.len() > MAX_NODES
            || sample.edges.len() > MAX_EDGES
            || sample.base_features.iter().any(|value| !value.is_finite())
            || sample
                .nodes
                .iter()
                .any(|node| node.len() != node_width || node.iter().any(|value| !value.is_finite()))
            || sample.edges.iter().any(|edge| {
                edge.source >= sample.nodes.len()
                    || edge.target >= sample.nodes.len()
                    || !edge.weight.is_finite()
            })
            || !sample.target.is_finite()
        {
            return Err(GraphRepresentationError::new(
                "graph samples are invalid or cross-split duplicated",
            ));
        }
    }
    Ok(node_width)
}

fn canonical_digest<T: Serialize>(
    domain: &[u8],
    value: &T,
) -> Result<Digest, GraphRepresentationError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| GraphRepresentationError::new(error.to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(bytes);
    Ok(Digest(hasher.finalize().into()))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphRepresentationError(String);

impl GraphRepresentationError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for GraphRepresentationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for GraphRepresentationError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn samples(start: u8, count: usize) -> Vec<GraphSample> {
        (0..count)
            .map(|index| {
                let positive = index % 2 == 0;
                GraphSample {
                    sample_sha256: Digest([start.wrapping_add(index as u8); 32]),
                    base_features: vec![0.0],
                    nodes: vec![vec![1.0], vec![2.0], vec![3.0]],
                    edges: if positive {
                        vec![
                            GraphEdge {
                                source: 2,
                                target: 0,
                                weight: 1.0,
                            },
                            GraphEdge {
                                source: 2,
                                target: 1,
                                weight: 1.0,
                            },
                        ]
                    } else {
                        vec![
                            GraphEdge {
                                source: 0,
                                target: 1,
                                weight: 1.0,
                            },
                            GraphEdge {
                                source: 0,
                                target: 2,
                                weight: 1.0,
                            },
                        ]
                    },
                    target: if positive { 1.0 } else { -1.0 },
                }
            })
            .collect()
    }

    #[test]
    fn topology_signal_is_compared_to_simpler_pooling_on_equal_rows() {
        let config = GraphComparisonConfig {
            minimum_training_samples: 20,
            minimum_held_out_samples: 10,
            ..GraphComparisonConfig::default()
        };
        for domain in [GraphDomain::ActorRelationships, GraphDomain::LocalCollision] {
            let report = GraphEncoderComparison::evaluate(
                domain,
                Digest([1; 32]),
                Digest([2; 32]),
                Digest([3; 32]),
                Digest([4; 32]),
                &samples(10, 20),
                &samples(100, 10),
                config,
            )
            .unwrap();
            assert!(report.equal_training_row_budget);
            assert!(report.equal_held_out_row_budget);
            assert!(report.message_passing.held_out_mse < report.simpler.held_out_mse * 0.1);
            assert_eq!(report.decision, GraphEncoderDecision::GraphEncoderCandidate);
            assert!(!report.promotion_authority);
            assert_ne!(report.comparison_sha256, Digest::ZERO);
        }
    }

    #[test]
    fn graph_comparison_rejects_cross_split_sample_identity() {
        let training = samples(1, 4);
        let mut held_out = samples(20, 2);
        held_out[0].sample_sha256 = training[0].sample_sha256;
        assert!(
            GraphEncoderComparison::evaluate(
                GraphDomain::ActorRelationships,
                Digest([1; 32]),
                Digest([2; 32]),
                Digest([3; 32]),
                Digest([4; 32]),
                &training,
                &held_out,
                GraphComparisonConfig {
                    minimum_training_samples: 4,
                    minimum_held_out_samples: 2,
                    ..GraphComparisonConfig::default()
                },
            )
            .is_err()
        );
    }
}
