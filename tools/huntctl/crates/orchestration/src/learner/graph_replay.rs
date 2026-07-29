use super::{
    ActionConditionedGraphLearner, ExactGraphTableLearner, ExactGraphTableSnapshot,
    GraphAuxiliaryPrediction, GraphExpansionLearningTarget, GraphLearnerContract,
    GraphLearnerError, GraphLearningBatch, GraphTargetSupport,
};
use dusklight_automation_contracts::artifact::Digest;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const GRAPH_REPLAY_PLAN_SCHEMA_V1: &str = "dusklight-graph-replay-plan/v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GraphReplayRowPriority {
    pub expansion_sha256: Digest,
    pub surprise_millionths: u32,
    pub rarity_millionths: u32,
    pub terminal_connected: bool,
    pub policy_relevant: bool,
    pub composite_priority_millionths: u32,
    pub ordinary_draws: u64,
    pub prioritized_draws: u64,
}

impl GraphReplayRowPriority {
    pub fn total_draws(&self) -> u64 {
        self.ordinary_draws.saturating_add(self.prioritized_draws)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GraphReplayPlan {
    pub schema: String,
    pub graph_sha256: Digest,
    pub contract_sha256: Digest,
    pub round: u64,
    pub source_rows: u64,
    pub policy_relevant_actions: Vec<Digest>,
    pub ordinary_draws: u64,
    pub prioritized_draws: u64,
    pub maximum_ordinary_starvation_rounds: u64,
    pub rows: Vec<GraphReplayRowPriority>,
    pub plan_sha256: Digest,
}

impl GraphReplayPlan {
    pub fn build(
        contract: &GraphLearnerContract,
        batch: &GraphLearningBatch,
        policy_relevant_actions: &BTreeSet<Digest>,
        round: u64,
    ) -> Result<Self, GraphLearnerError> {
        contract.validate()?;
        batch.validate()?;
        if batch.rows.is_empty() {
            return Err(GraphLearnerError::Invalid(
                "graph replay requires at least one realized edge",
            ));
        }
        let learner = ExactGraphTableLearner;
        let snapshot = learner.fit(contract, batch)?;
        let action_counts = action_counts(batch)?;
        let mut rows = batch
            .rows
            .iter()
            .map(|row| {
                priority_for_row(
                    contract,
                    row,
                    &snapshot,
                    &action_counts,
                    policy_relevant_actions,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        rows.sort_by_key(|row| row.expansion_sha256);

        let total_draws = contract.minimum_replay_rows.max(rows.len() as u64);
        let ordinary_draws = u64::try_from(
            u128::from(total_draws)
                .saturating_mul(u128::from(contract.ordinary_replay_fraction_per_million))
                .div_ceil(1_000_000),
        )
        .unwrap_or(u64::MAX)
        .clamp(1, total_draws.saturating_sub(1).max(1));
        let prioritized_draws = total_draws.saturating_sub(ordinary_draws);
        let ordinary_offset = usize::try_from(
            u128::from(round).saturating_mul(u128::from(ordinary_draws)) % rows.len() as u128,
        )
        .unwrap_or(0);
        for draw in 0..ordinary_draws {
            let index = (ordinary_offset + draw as usize % rows.len()) % rows.len();
            rows[index].ordinary_draws = rows[index].ordinary_draws.saturating_add(1);
        }
        allocate_prioritized_draws(&mut rows, prioritized_draws);
        let maximum_ordinary_starvation_rounds =
            (rows.len() as u64).div_ceil(ordinary_draws).max(1);
        let mut plan = Self {
            schema: GRAPH_REPLAY_PLAN_SCHEMA_V1.into(),
            graph_sha256: batch.graph_sha256,
            contract_sha256: contract.content_sha256()?,
            round,
            source_rows: rows.len() as u64,
            policy_relevant_actions: policy_relevant_actions.iter().copied().collect(),
            ordinary_draws,
            prioritized_draws,
            maximum_ordinary_starvation_rounds,
            rows,
            plan_sha256: Digest::ZERO,
        };
        plan.plan_sha256 = plan.digest()?;
        plan.validate(contract, batch)?;
        Ok(plan)
    }

    pub fn draw_weight(&self, expansion_sha256: Digest) -> u64 {
        self.rows
            .iter()
            .find(|row| row.expansion_sha256 == expansion_sha256)
            .map_or(0, GraphReplayRowPriority::total_draws)
    }

    pub fn validate(
        &self,
        contract: &GraphLearnerContract,
        batch: &GraphLearningBatch,
    ) -> Result<(), GraphLearnerError> {
        contract.validate()?;
        batch.validate()?;
        let batch_expansions = batch
            .rows
            .iter()
            .map(|row| row.expansion_sha256)
            .collect::<BTreeSet<_>>();
        let plan_expansions = self
            .rows
            .iter()
            .map(|row| row.expansion_sha256)
            .collect::<BTreeSet<_>>();
        let batch_rows = batch
            .rows
            .iter()
            .map(|row| (row.expansion_sha256, row))
            .collect::<BTreeMap<_, _>>();
        let policy_relevant_actions = self
            .policy_relevant_actions
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        let action_counts = action_counts(batch)?;
        let snapshot = ExactGraphTableLearner.fit(contract, batch)?;
        let expected_priorities = batch
            .rows
            .iter()
            .map(|row| {
                Ok((
                    row.expansion_sha256,
                    priority_for_row(
                        contract,
                        row,
                        &snapshot,
                        &action_counts,
                        &policy_relevant_actions,
                    )?,
                ))
            })
            .collect::<Result<BTreeMap<_, _>, GraphLearnerError>>()?;
        let ordinary_sum = self.rows.iter().map(|row| row.ordinary_draws).sum::<u64>();
        let prioritized_sum = self
            .rows
            .iter()
            .map(|row| row.prioritized_draws)
            .sum::<u64>();
        let mut expected_draws = self.rows.clone();
        for row in &mut expected_draws {
            row.ordinary_draws = 0;
            row.prioritized_draws = 0;
        }
        let ordinary_offset = usize::try_from(
            u128::from(self.round).saturating_mul(u128::from(self.ordinary_draws))
                % expected_draws.len().max(1) as u128,
        )
        .unwrap_or(0);
        for draw in 0..self.ordinary_draws {
            let index =
                (ordinary_offset + draw as usize % expected_draws.len()) % expected_draws.len();
            expected_draws[index].ordinary_draws =
                expected_draws[index].ordinary_draws.saturating_add(1);
        }
        allocate_prioritized_draws(&mut expected_draws, self.prioritized_draws);
        if self.schema != GRAPH_REPLAY_PLAN_SCHEMA_V1
            || self.graph_sha256 != batch.graph_sha256
            || self.contract_sha256 != contract.content_sha256()?
            || self.source_rows != batch.rows.len() as u64
            || self.rows.len() != plan_expansions.len()
            || self.policy_relevant_actions.len() != policy_relevant_actions.len()
            || self
                .policy_relevant_actions
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
            || self
                .rows
                .windows(2)
                .any(|pair| pair[0].expansion_sha256 >= pair[1].expansion_sha256)
            || batch_expansions != plan_expansions
            || self.ordinary_draws == 0
            || self.prioritized_draws == 0
            || ordinary_sum != self.ordinary_draws
            || prioritized_sum != self.prioritized_draws
            || self.maximum_ordinary_starvation_rounds
                != self.source_rows.div_ceil(self.ordinary_draws).max(1)
            || self
                .rows
                .iter()
                .zip(&expected_draws)
                .any(|(actual, expected)| {
                    actual.ordinary_draws != expected.ordinary_draws
                        || actual.prioritized_draws != expected.prioritized_draws
                })
            || self.rows.iter().any(|row| {
                let source = batch_rows[&row.expansion_sha256];
                let expected = &expected_priorities[&row.expansion_sha256];
                row.surprise_millionths > 1_000_000
                    || row.surprise_millionths != expected.surprise_millionths
                    || row.rarity_millionths != expected.rarity_millionths
                    || row.terminal_connected
                        != (source.support == GraphTargetSupport::ExactTerminalPath)
                    || row.policy_relevant != expected.policy_relevant
                    || row.composite_priority_millionths != expected.composite_priority_millionths
            })
            || self.plan_sha256 == Digest::ZERO
            || self.plan_sha256 != self.digest()?
        {
            return Err(GraphLearnerError::Invalid("graph replay plan is detached"));
        }
        Ok(())
    }

    fn digest(&self) -> Result<Digest, GraphLearnerError> {
        let mut canonical = self.clone();
        canonical.plan_sha256 = Digest::ZERO;
        let raw = serde_cbor::to_vec(&canonical)
            .map_err(|error| GraphLearnerError::Serialization(error.to_string()))?;
        let mut hasher = Sha256::new();
        hasher.update(GRAPH_REPLAY_PLAN_SCHEMA_V1.as_bytes());
        hasher.update(raw);
        Ok(Digest(hasher.finalize().into()))
    }
}

fn action_counts(batch: &GraphLearningBatch) -> Result<BTreeMap<Digest, u64>, GraphLearnerError> {
    let mut counts = BTreeMap::new();
    for row in &batch.rows {
        let action = row
            .action
            .content_sha256()
            .map_err(|error| GraphLearnerError::Action(error.to_string()))?;
        *counts.entry(action).or_default() += 1;
    }
    Ok(counts)
}

fn priority_for_row(
    contract: &GraphLearnerContract,
    row: &GraphExpansionLearningTarget,
    snapshot: &ExactGraphTableSnapshot,
    action_counts: &BTreeMap<Digest, u64>,
    policy_relevant_actions: &BTreeSet<Digest>,
) -> Result<GraphReplayRowPriority, GraphLearnerError> {
    let action_sha256 = row
        .action
        .content_sha256()
        .map_err(|error| GraphLearnerError::Action(error.to_string()))?;
    let auxiliary_surprise = snapshot
        .generalized_auxiliary_prediction(&row.source_features, action_sha256)
        .map(|prediction| auxiliary_error(row, &prediction));
    let objective_surprise = (row.support == GraphTargetSupport::ExactTerminalPath)
        .then(|| {
            snapshot
                .generalized_objective_prediction(&row.source_features, action_sha256)
                .map(|prediction| prediction.prediction_error_millionths)
        })
        .flatten();
    let surprise_millionths = match (auxiliary_surprise, objective_surprise) {
        (Some(left), Some(right)) => ((left / 2).saturating_add(right / 2)).min(1_000_000),
        (Some(value), None) | (None, Some(value)) => value.min(1_000_000),
        (None, None) => 0,
    } as u32;
    let rarity_millionths = rarity_for_row(row, action_counts);
    let terminal_connected = row.support == GraphTargetSupport::ExactTerminalPath;
    let policy_relevant = policy_relevant_actions.contains(&action_sha256);
    let composite_priority_millionths = weighted_priority(
        contract,
        surprise_millionths,
        rarity_millionths,
        terminal_connected,
        policy_relevant,
    );
    Ok(GraphReplayRowPriority {
        expansion_sha256: row.expansion_sha256,
        surprise_millionths,
        rarity_millionths,
        terminal_connected,
        policy_relevant,
        composite_priority_millionths,
        ordinary_draws: 0,
        prioritized_draws: 0,
    })
}

fn rarity_for_row(
    row: &GraphExpansionLearningTarget,
    action_counts: &BTreeMap<Digest, u64>,
) -> u32 {
    let action_sha256 = row
        .action
        .content_sha256()
        .expect("validated replay action has a stable identity");
    let action_count = action_counts.get(&action_sha256).copied().unwrap_or(1);
    ((1_000_000 / action_count.max(1)).saturating_add(1_000_000 / row.graph_visits.max(1)) / 2)
        as u32
}

fn weighted_priority(
    contract: &GraphLearnerContract,
    surprise: u32,
    rarity: u32,
    terminal: bool,
    policy: bool,
) -> u32 {
    let value = u128::from(surprise) * u128::from(contract.replay_surprise_weight_per_million)
        + u128::from(rarity) * u128::from(contract.replay_rarity_weight_per_million)
        + u128::from(u32::from(terminal) * 1_000_000)
            * u128::from(contract.replay_terminal_weight_per_million)
        + u128::from(u32::from(policy) * 1_000_000)
            * u128::from(contract.replay_policy_weight_per_million);
    u32::try_from(value / 1_000_000).unwrap_or(1_000_000)
}

fn allocate_prioritized_draws(rows: &mut [GraphReplayRowPriority], draws: u64) {
    for _ in 0..draws {
        let selected = (0..rows.len())
            .max_by(|left, right| {
                let left_score = u128::from(rows[*left].composite_priority_millionths.max(1))
                    * u128::from(rows[*right].prioritized_draws.saturating_add(1));
                let right_score = u128::from(rows[*right].composite_priority_millionths.max(1))
                    * u128::from(rows[*left].prioritized_draws.saturating_add(1));
                left_score.cmp(&right_score).then_with(|| {
                    rows[*right]
                        .expansion_sha256
                        .cmp(&rows[*left].expansion_sha256)
                })
            })
            .expect("validated replay rows are nonempty");
        rows[selected].prioritized_draws = rows[selected].prioritized_draws.saturating_add(1);
    }
}

fn auxiliary_error(
    row: &GraphExpansionLearningTarget,
    prediction: &GraphAuxiliaryPrediction,
) -> u64 {
    let duration_scale = u64::from(
        row.realized_duration_ticks
            .max(prediction.realized_duration_ticks)
            .max(1),
    );
    let duration = u64::from(
        row.realized_duration_ticks
            .abs_diff(prediction.realized_duration_ticks),
    )
    .saturating_mul(1_000_000)
        / duration_scale;
    let acceptance = u64::from(
        prediction
            .action_acceptance_per_million
            .abs_diff(u32::from(row.action_accepted) * 1_000_000),
    );
    let terminal = u64::from(
        prediction
            .immediate_terminal_per_million
            .abs_diff(u32::from(row.immediate_terminal) * 1_000_000),
    );
    let prompt =
        u64::from(prediction.prompted_action_status != row.prompted_action_status) * 1_000_000;
    let features = if prediction.next_state_feature_f32_bits.len() != row.target_features.len() {
        1_000_000
    } else {
        let error = row
            .target_features
            .iter()
            .zip(&prediction.next_state_feature_f32_bits)
            .map(|(actual, predicted)| {
                let predicted = f32::from_bits(*predicted);
                ((*actual - predicted).abs() / (actual.abs() + 1.0)).min(1.0)
            })
            .sum::<f32>()
            / row.target_features.len().max(1) as f32;
        (error * 1_000_000.0).round() as u64
    };
    duration
        .saturating_add(acceptance)
        .saturating_add(terminal)
        .saturating_add(prompt)
        .saturating_add(features)
        / 5
}
