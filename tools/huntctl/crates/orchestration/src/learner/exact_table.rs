use super::{
    ActionConditionedGraphLearner, GraphActionInput, GraphLearnerContract, GraphLearnerError,
    GraphLearningBatch, GraphNodeInput, GraphTargetSupport, LearnedGraphActionEstimate,
};
use crate::state_graph::ExactStateId;
use dusklight_automation_contracts::artifact::Digest;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default)]
pub struct ExactGraphTableLearner;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExactGraphTableSnapshot {
    pub contract_sha256: Digest,
    pub graph_sha256: Digest,
    estimates: BTreeMap<(ExactStateId, Digest), LearnedGraphActionEstimate>,
}

impl ExactGraphTableSnapshot {
    pub fn estimate(
        &self,
        state: ExactStateId,
        action_sha256: Digest,
    ) -> Option<LearnedGraphActionEstimate> {
        self.estimates.get(&(state, action_sha256)).copied()
    }

    pub fn len(&self) -> usize {
        self.estimates.len()
    }

    pub fn is_empty(&self) -> bool {
        self.estimates.is_empty()
    }
}

impl ActionConditionedGraphLearner for ExactGraphTableLearner {
    type Snapshot = ExactGraphTableSnapshot;

    fn fit(
        &self,
        contract: &GraphLearnerContract,
        batch: &GraphLearningBatch,
    ) -> Result<Self::Snapshot, GraphLearnerError> {
        contract.validate()?;
        batch.validate()?;
        let mut estimates = BTreeMap::new();
        for row in &batch.rows {
            let action_sha256 = row
                .action
                .content_sha256()
                .map_err(|error| GraphLearnerError::Action(error.to_string()))?;
            let estimate = match row.support {
                GraphTargetSupport::ExactTerminalPath => LearnedGraphActionEstimate {
                    terminal_support_per_million: Some(1_000_000),
                    conditional_ticks_to_terminal: row.exact_conditional_ticks_to_terminal,
                    uncertainty_millionths: inverse_visit_uncertainty(row.graph_visits),
                    prediction_error_millionths: 0,
                },
                GraphTargetSupport::OpenContinuationCensored => LearnedGraphActionEstimate {
                    terminal_support_per_million: None,
                    conditional_ticks_to_terminal: None,
                    uncertainty_millionths: inverse_visit_uncertainty(row.graph_visits),
                    prediction_error_millionths: 0,
                },
            };
            if estimates
                .insert((row.source, action_sha256), estimate)
                .is_some()
            {
                return Err(GraphLearnerError::Invalid(
                    "exact learner received duplicate state/action targets",
                ));
            }
        }
        Ok(ExactGraphTableSnapshot {
            contract_sha256: contract.content_sha256()?,
            graph_sha256: batch.graph_sha256,
            estimates,
        })
    }

    fn rank(
        &self,
        snapshot: &Self::Snapshot,
        node: &GraphNodeInput,
        actions: &[GraphActionInput],
    ) -> Result<Vec<LearnedGraphActionEstimate>, GraphLearnerError> {
        node.state
            .validate()
            .map_err(|error| GraphLearnerError::Facts(error.to_string()))?;
        if node
            .state
            .content_sha256()
            .map_err(|error| GraphLearnerError::Facts(error.to_string()))?
            != node.id.state_sha256
            || node.graph_visits == 0
        {
            return Err(GraphLearnerError::Invalid(
                "exact learner node input is detached",
            ));
        }
        actions
            .iter()
            .map(|action| {
                action
                    .action
                    .validate()
                    .map_err(|error| GraphLearnerError::Action(error.to_string()))?;
                if action.expansion_sha256 == Digest::ZERO {
                    return Err(GraphLearnerError::Invalid(
                        "exact learner action identity is missing",
                    ));
                }
                let action_sha256 = action
                    .action
                    .content_sha256()
                    .map_err(|error| GraphLearnerError::Action(error.to_string()))?;
                Ok(snapshot.estimate(node.id, action_sha256).unwrap_or(
                    LearnedGraphActionEstimate {
                        terminal_support_per_million: None,
                        conditional_ticks_to_terminal: None,
                        uncertainty_millionths: u64::MAX,
                        prediction_error_millionths: 0,
                    },
                ))
            })
            .collect()
    }
}

fn inverse_visit_uncertainty(visits: u64) -> u64 {
    1_000_000_u64 / visits.max(1)
}
