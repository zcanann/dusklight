use super::{
    GraphLearnerError, GraphLearningBatch, GraphTargetSupport, LearnedGraphActionEstimate,
};
use dusklight_automation_contracts::artifact::Digest;
use dusklight_learning::double_q::{DoubleQ, DoubleQConfig};
use dusklight_learning::fqi::Transition;
use std::collections::{BTreeMap, BTreeSet};

const GRAPH_DOUBLE_Q_SEED: u64 = 0x4752_4150_4844_5101;

#[derive(Clone, Debug)]
pub(super) struct GraphDoubleQObjectiveModel {
    model: DoubleQ,
    action_ids: BTreeMap<Digest, u32>,
}

impl GraphDoubleQObjectiveModel {
    pub(super) fn fit(batch: &GraphLearningBatch) -> Result<Option<Self>, GraphLearnerError> {
        batch.validate()?;
        let supported_actions = batch
            .rows
            .iter()
            .filter(|row| row.support == GraphTargetSupport::ExactTerminalPath)
            .map(|row| {
                row.action
                    .content_sha256()
                    .map_err(|error| GraphLearnerError::Action(error.to_string()))
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        if supported_actions.is_empty() {
            return Ok(None);
        }
        let action_ids = supported_actions
            .into_iter()
            .enumerate()
            .map(|(index, action)| {
                Ok((
                    action,
                    u32::try_from(index).map_err(|_| {
                        GraphLearnerError::Invalid("graph Double-Q action count exceeds u32")
                    })?,
                ))
            })
            .collect::<Result<BTreeMap<_, _>, GraphLearnerError>>()?;
        let mut transitions = Vec::new();
        for row in &batch.rows {
            let Some(ticks) = row.exact_conditional_ticks_to_terminal else {
                continue;
            };
            let action_sha256 = row
                .action
                .content_sha256()
                .map_err(|error| GraphLearnerError::Action(error.to_string()))?;
            let reward = -(ticks as f32);
            if !reward.is_finite() {
                return Err(GraphLearnerError::Invalid(
                    "graph Double-Q target is outside finite f32",
                ));
            }
            transitions.push(Transition {
                state: row.source_features.clone(),
                action: action_ids[&action_sha256],
                duration: row.realized_duration_ticks,
                reward,
                next_state: row.source_features.clone(),
                terminal: true,
            });
        }
        if transitions.len() < 2 {
            return Ok(None);
        }
        let feature_width = transitions[0].state.len();
        let actions = action_ids.values().copied().collect::<Vec<_>>();
        let model = DoubleQ::fit(
            feature_width,
            &actions,
            &transitions,
            &DoubleQConfig {
                epochs: 128,
                hidden_width: 32,
                learning_rate: 0.003,
                discount: 1.0,
                target_sync_steps: 64,
                gradient_clip: 10.0,
                seed: GRAPH_DOUBLE_Q_SEED,
            },
        )
        .map_err(|error| GraphLearnerError::Model(error.to_string()))?;
        Ok(Some(Self { model, action_ids }))
    }

    pub(super) fn predict(
        &self,
        source_features: &[f32],
        action_sha256: Digest,
    ) -> Option<LearnedGraphActionEstimate> {
        let action = self.action_ids.get(&action_sha256)?;
        let estimate = self.model.estimate(source_features, *action).ok()?;
        if !estimate.mean.is_finite() || !estimate.critic_disagreement.is_finite() {
            return None;
        }
        let ticks = (-estimate.mean).round().clamp(0.0, u64::MAX as f64) as u64;
        let uncertainty_millionths =
            (estimate.critic_disagreement.abs() / (-estimate.mean).abs().max(1.0) * 1_000_000.0)
                .round()
                .clamp(0.0, u64::MAX as f64) as u64;
        Some(LearnedGraphActionEstimate {
            terminal_support_per_million: None,
            conditional_ticks_to_terminal: Some(ticks),
            uncertainty_millionths,
            prediction_error_millionths: 0,
        })
    }
}
