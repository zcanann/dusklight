use crate::state_graph::{
    ActionExpansionStatus, ExactStateId, ExpansionEvidenceAuthority, StateGraph, StateGraphError,
};
use dusklight_automation_contracts::artifact::Digest;
use dusklight_learning::fact_snapshot::FactSnapshot;
use dusklight_learning::option_values::OptionActionDescriptor;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;

pub const GRAPH_LEARNING_BATCH_SCHEMA_V1: &str = "dusklight-graph-learning-batch/v1";
pub const GRAPH_LEARNER_CONTRACT_SCHEMA_V1: &str = "dusklight-graph-learner-contract/v1";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphBootstrapRule {
    ExactMonteCarloThenCensoredNStep,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphUncertaintyRule {
    HeldOutBootstrapEnsembleVariance,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphRankingTuple {
    TerminalSupportConditionalTicksUncertaintyVisits,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GraphLearnerContract {
    pub schema: String,
    pub bootstrap_rule: GraphBootstrapRule,
    pub n_step_horizon_ticks: u32,
    pub uncertainty_rule: GraphUncertaintyRule,
    pub ensemble_members: u32,
    pub target_network_update_every_batches: u64,
    pub minimum_replay_rows: u64,
    pub ranking_tuple: GraphRankingTuple,
}

impl Default for GraphLearnerContract {
    fn default() -> Self {
        Self {
            schema: GRAPH_LEARNER_CONTRACT_SCHEMA_V1.into(),
            bootstrap_rule: GraphBootstrapRule::ExactMonteCarloThenCensoredNStep,
            n_step_horizon_ticks: 16,
            uncertainty_rule: GraphUncertaintyRule::HeldOutBootstrapEnsembleVariance,
            ensemble_members: 5,
            target_network_update_every_batches: 32,
            minimum_replay_rows: 64,
            ranking_tuple: GraphRankingTuple::TerminalSupportConditionalTicksUncertaintyVisits,
        }
    }
}

impl GraphLearnerContract {
    pub fn validate(&self) -> Result<(), GraphLearnerError> {
        if self.schema != GRAPH_LEARNER_CONTRACT_SCHEMA_V1
            || self.n_step_horizon_ticks == 0
            || self.n_step_horizon_ticks > 256
            || self.ensemble_members < 2
            || self.ensemble_members > 64
            || self.target_network_update_every_batches == 0
            || self.minimum_replay_rows == 0
        {
            return Err(GraphLearnerError::Invalid(
                "graph learner algorithm contract is invalid",
            ));
        }
        Ok(())
    }

    pub fn content_sha256(&self) -> Result<Digest, GraphLearnerError> {
        self.validate()?;
        let mut hasher = Sha256::new();
        hasher.update(GRAPH_LEARNER_CONTRACT_SCHEMA_V1.as_bytes());
        hasher.update([self.bootstrap_rule as u8]);
        hasher.update(self.n_step_horizon_ticks.to_le_bytes());
        hasher.update([self.uncertainty_rule as u8]);
        hasher.update(self.ensemble_members.to_le_bytes());
        hasher.update(self.target_network_update_every_batches.to_le_bytes());
        hasher.update(self.minimum_replay_rows.to_le_bytes());
        hasher.update([self.ranking_tuple as u8]);
        Ok(Digest(hasher.finalize().into()))
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphTargetSupport {
    /// This exact realized expansion reconnects to an authenticated terminal
    /// tape. Conditional ticks are an exact Monte Carlo target.
    ExactTerminalPath,
    /// Native execution ended at an open nonterminal boundary. This is
    /// right-censored evidence, not a negative terminal label.
    OpenContinuationCensored,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GraphExpansionLearningTarget {
    pub expansion_sha256: Digest,
    pub source: ExactStateId,
    pub target: ExactStateId,
    pub source_state: FactSnapshot,
    pub action: OptionActionDescriptor,
    pub realized_duration_ticks: u32,
    pub graph_visits: u64,
    pub support: GraphTargetSupport,
    pub exact_conditional_ticks_to_terminal: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GraphLearningBatch {
    pub schema: String,
    pub graph_sha256: Digest,
    pub rows: Vec<GraphExpansionLearningTarget>,
}

impl GraphLearningBatch {
    pub fn from_graph(graph: &StateGraph) -> Result<Self, GraphLearnerError> {
        graph.validate()?;
        let exact_returns = graph.exact_terminal_returns()?;
        let mut rows = Vec::new();
        for expansion in graph.expansions() {
            let ActionExpansionStatus::Completed {
                authority,
                evidence,
                ..
            } = &expansion.status
            else {
                continue;
            };
            if *authority != ExpansionEvidenceAuthority::Executable {
                continue;
            }
            let target = expansion.target.ok_or(GraphLearnerError::Invalid(
                "completed expansion has no target",
            ))?;
            let execution = expansion
                .execution
                .as_ref()
                .ok_or(GraphLearnerError::Invalid(
                    "completed expansion has no execution",
                ))?;
            let source_state = graph
                .node(expansion.source)
                .ok_or(GraphLearnerError::Invalid(
                    "completed expansion source is absent",
                ))?
                .state
                .clone();
            let exact_conditional_ticks_to_terminal = exact_returns
                .get(&target)
                .map(|ticks| u64::from(execution.duration.realized_ticks).saturating_add(*ticks));
            rows.push(GraphExpansionLearningTarget {
                expansion_sha256: expansion.identity_sha256,
                source: expansion.source,
                target,
                source_state,
                action: expansion.action.clone(),
                realized_duration_ticks: execution.duration.realized_ticks,
                graph_visits: evidence.len() as u64,
                support: if exact_conditional_ticks_to_terminal.is_some() {
                    GraphTargetSupport::ExactTerminalPath
                } else {
                    GraphTargetSupport::OpenContinuationCensored
                },
                exact_conditional_ticks_to_terminal,
            });
        }
        let batch = Self {
            schema: GRAPH_LEARNING_BATCH_SCHEMA_V1.into(),
            graph_sha256: graph.content_sha256()?,
            rows,
        };
        batch.validate()?;
        Ok(batch)
    }

    pub fn validate(&self) -> Result<(), GraphLearnerError> {
        if self.schema != GRAPH_LEARNING_BATCH_SCHEMA_V1 || self.graph_sha256 == Digest::ZERO {
            return Err(GraphLearnerError::Invalid(
                "graph learning batch identity is invalid",
            ));
        }
        for row in &self.rows {
            row.source_state
                .validate()
                .map_err(|error| GraphLearnerError::Facts(error.to_string()))?;
            row.action
                .validate()
                .map_err(|error| GraphLearnerError::Action(error.to_string()))?;
            if row.expansion_sha256 == Digest::ZERO
                || row.realized_duration_ticks == 0
                || row.graph_visits == 0
                || row
                    .source_state
                    .content_sha256()
                    .map_err(|error| GraphLearnerError::Facts(error.to_string()))?
                    != row.source.state_sha256
                || (row.support == GraphTargetSupport::ExactTerminalPath)
                    != row.exact_conditional_ticks_to_terminal.is_some()
                || row
                    .exact_conditional_ticks_to_terminal
                    .is_some_and(|ticks| ticks < u64::from(row.realized_duration_ticks))
            {
                return Err(GraphLearnerError::Invalid(
                    "graph learning row is internally detached",
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct GraphNodeInput {
    pub id: ExactStateId,
    pub state: FactSnapshot,
    pub graph_visits: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphActionInput {
    pub expansion_sha256: Digest,
    pub action: OptionActionDescriptor,
    pub graph_visits: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LearnedGraphActionEstimate {
    pub terminal_support_per_million: Option<u32>,
    pub conditional_ticks_to_terminal: Option<u64>,
    pub uncertainty_millionths: u64,
    pub prediction_error_millionths: u64,
}

/// One learner surface for every primitive and promoted option. Implementations
/// may change, but scheduling consumes only these separately inspectable heads.
pub trait ActionConditionedGraphLearner {
    type Snapshot;

    fn fit(
        &self,
        contract: &GraphLearnerContract,
        batch: &GraphLearningBatch,
    ) -> Result<Self::Snapshot, GraphLearnerError>;

    fn rank(
        &self,
        snapshot: &Self::Snapshot,
        node: &GraphNodeInput,
        actions: &[GraphActionInput],
    ) -> Result<Vec<LearnedGraphActionEstimate>, GraphLearnerError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn algorithm_contract_is_sealed_and_rejects_implicit_cadence() {
        let contract = GraphLearnerContract::default();
        let identity = contract.content_sha256().unwrap();
        let mut changed = contract.clone();
        changed.target_network_update_every_batches += 1;
        assert_ne!(changed.content_sha256().unwrap(), identity);

        let mut invalid = contract;
        invalid.target_network_update_every_batches = 0;
        assert!(invalid.validate().is_err());
    }
}

#[derive(Debug)]
pub enum GraphLearnerError {
    Invalid(&'static str),
    Facts(String),
    Action(String),
    Graph(StateGraphError),
}

impl fmt::Display for GraphLearnerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) => {
                write!(formatter, "invalid graph learner contract: {message}")
            }
            Self::Facts(message) => write!(formatter, "graph learner facts failed: {message}"),
            Self::Action(message) => write!(formatter, "graph learner action failed: {message}"),
            Self::Graph(error) => write!(formatter, "graph learner state graph failed: {error}"),
        }
    }
}

impl Error for GraphLearnerError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Graph(error) => Some(error),
            _ => None,
        }
    }
}

impl From<StateGraphError> for GraphLearnerError {
    fn from(value: StateGraphError) -> Self {
        Self::Graph(value)
    }
}
