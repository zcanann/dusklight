//! Deterministic scheduling over graph-owned expansion lifecycle state.
//!
//! The scheduler ranks work but never owns it. Leasing mutates the state graph,
//! which supplies virtual loss to every worker sharing that graph.

use crate::state_graph::{ExactStateId, StateGraph, StateGraphError, ValidatedStateGraph};
use dusklight_automation_contracts::artifact::Digest;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

pub const EXPANSION_SCHEDULER_CONFIG_SCHEMA_V1: &str = "dusklight-expansion-scheduler-config/v1";
pub const GRAPH_PRIORITY_SNAPSHOT_SCHEMA_V1: &str = "dusklight-graph-priority-snapshot/v1";
pub const REPLAYABLE_EXPANSION_QUEUE_SCHEMA_V1: &str = "dusklight-replayable-expansion-queue/v1";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchRegime {
    Discovery,
    Optimization,
}

/// Integer learner outputs keep queue ordering stable across platforms.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LearnedExpansionPriority {
    /// Stable order supplied by the active exploration/value policy. This is
    /// a fallback while terminal-support and conditional-return heads are
    /// unavailable, not an alternative source of expansion ownership.
    pub policy_rank: Option<u64>,
    pub terminal_support_per_million: Option<u32>,
    pub conditional_ticks_to_go: Option<u64>,
    pub uncertainty_millionths: u64,
    pub prediction_error_millionths: u64,
    pub completed_visits: u64,
}

impl LearnedExpansionPriority {
    pub fn validate(&self) -> Result<(), SchedulerError> {
        if self
            .terminal_support_per_million
            .is_some_and(|value| value > 1_000_000)
        {
            return Err(SchedulerError::Invalid(
                "terminal support exceeds one million",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScheduledExpansion {
    pub expansion_sha256: Digest,
    pub source: ExactStateId,
    pub source_root_ticks: u64,
    /// Exact authenticated cost from this source through the current best
    /// terminal route. `None` is explicit absence of exact terminal support.
    pub source_exact_terminal_ticks_to_go: Option<u64>,
    /// Action-conditioned estimate learned across realized graph evidence.
    /// This never substitutes for the exact source return above.
    pub generalized_conditional_ticks_to_go: Option<u64>,
    /// Separately published epistemic/exploration signal.
    pub uncertainty_millionths: u64,
    /// Zero-based position in the complete deterministic expansion queue.
    pub exploration_priority_rank: u64,
    pub learned: LearnedExpansionPriority,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScheduledNode {
    pub node: ExactStateId,
    pub root_ticks: u64,
    pub registered_expansions: u64,
    pub completed_expansions: u64,
    /// Squared, quantized world-state distance to the nearest graph node that
    /// has already owned expansion work. This is derived from exact graph
    /// states and is used only before terminal support exists.
    pub reachability_novelty: u128,
    pub exact_terminal_ticks_to_go: Option<u64>,
    pub tie_rank: Digest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExpansionSchedulerConfig {
    pub schema: String,
    pub regime: SearchRegime,
    pub seed: u64,
    pub generation: u64,
    pub lease_generations: u64,
}

impl ExpansionSchedulerConfig {
    pub fn validate(&self) -> Result<(), SchedulerError> {
        if self.schema != EXPANSION_SCHEDULER_CONFIG_SCHEMA_V1 || self.lease_generations == 0 {
            return Err(SchedulerError::Invalid(
                "sealed expansion scheduler config is invalid",
            ));
        }
        Ok(())
    }

    pub fn content_sha256(&self) -> Result<Digest, SchedulerError> {
        self.validate()?;
        let mut hasher = Sha256::new();
        hasher.update(EXPANSION_SCHEDULER_CONFIG_SCHEMA_V1.as_bytes());
        hasher.update([self.regime as u8]);
        hasher.update(self.seed.to_le_bytes());
        hasher.update(self.generation.to_le_bytes());
        hasher.update(self.lease_generations.to_le_bytes());
        Ok(Digest(hasher.finalize().into()))
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GraphPrioritySnapshot {
    pub schema: String,
    pub graph_sha256: Digest,
    /// Identity of the model or exact table that published these estimates.
    /// `None` is an explicit cold-start snapshot, not an implicit mutable map.
    pub learner_snapshot_sha256: Option<Digest>,
    pub estimates: BTreeMap<Digest, LearnedExpansionPriority>,
}

impl GraphPrioritySnapshot {
    pub fn cold_start(graph: &StateGraph) -> Result<Self, SchedulerError> {
        Ok(Self {
            schema: GRAPH_PRIORITY_SNAPSHOT_SCHEMA_V1.into(),
            graph_sha256: graph.content_sha256()?,
            learner_snapshot_sha256: None,
            estimates: BTreeMap::new(),
        })
    }

    pub fn validate_against(&self, graph: &StateGraph) -> Result<(), SchedulerError> {
        if self.schema != GRAPH_PRIORITY_SNAPSHOT_SCHEMA_V1
            || self.graph_sha256 != graph.content_sha256()?
            || self.learner_snapshot_sha256 == Some(Digest::ZERO)
        {
            return Err(SchedulerError::Invalid(
                "learner priority snapshot is detached from the graph",
            ));
        }
        for (expansion_sha256, estimate) in &self.estimates {
            if graph.expansion(*expansion_sha256).is_none() {
                return Err(SchedulerError::Invalid(
                    "learner priority snapshot names an absent expansion",
                ));
            }
            estimate.validate()?;
        }
        Ok(())
    }

    pub fn content_sha256(&self) -> Result<Digest, SchedulerError> {
        let mut hasher = Sha256::new();
        hasher.update(GRAPH_PRIORITY_SNAPSHOT_SCHEMA_V1.as_bytes());
        hasher.update(self.graph_sha256.0);
        match self.learner_snapshot_sha256 {
            Some(identity) => {
                hasher.update([1]);
                hasher.update(identity.0);
            }
            None => hasher.update([0]),
        }
        hasher.update((self.estimates.len() as u64).to_le_bytes());
        for (identity, estimate) in &self.estimates {
            estimate.validate()?;
            hasher.update(identity.0);
            hash_learned_priority(&mut hasher, *estimate);
        }
        Ok(Digest(hasher.finalize().into()))
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReplayableExpansionQueue {
    pub schema: String,
    pub graph_sha256: Digest,
    pub learner_priority_snapshot_sha256: Digest,
    pub scheduler_config_sha256: Digest,
    pub ranked_expansions: Vec<Digest>,
    pub selected_expansion_sha256: Option<Digest>,
    pub queue_sha256: Digest,
}

impl ReplayableExpansionQueue {
    fn seal(
        graph_sha256: Digest,
        learner_priority_snapshot_sha256: Digest,
        scheduler_config_sha256: Digest,
        ranked_expansions: Vec<Digest>,
    ) -> Self {
        let selected_expansion_sha256 = ranked_expansions.first().copied();
        let queue_sha256 = expansion_queue_sha256(
            graph_sha256,
            learner_priority_snapshot_sha256,
            scheduler_config_sha256,
            &ranked_expansions,
        );
        Self {
            schema: REPLAYABLE_EXPANSION_QUEUE_SCHEMA_V1.into(),
            graph_sha256,
            learner_priority_snapshot_sha256,
            scheduler_config_sha256,
            ranked_expansions,
            selected_expansion_sha256,
            queue_sha256,
        }
    }

    pub fn validate(&self) -> Result<(), SchedulerError> {
        if self.schema != REPLAYABLE_EXPANSION_QUEUE_SCHEMA_V1
            || self.selected_expansion_sha256 != self.ranked_expansions.first().copied()
            || self.queue_sha256
                != expansion_queue_sha256(
                    self.graph_sha256,
                    self.learner_priority_snapshot_sha256,
                    self.scheduler_config_sha256,
                    &self.ranked_expansions,
                )
        {
            return Err(SchedulerError::Invalid(
                "replayable expansion queue is invalid",
            ));
        }
        Ok(())
    }
}

pub fn replay_expansion_queue(
    graph: &StateGraph,
    config: &ExpansionSchedulerConfig,
    learned: &GraphPrioritySnapshot,
) -> Result<ReplayableExpansionQueue, SchedulerError> {
    graph.validate()?;
    config.validate()?;
    learned.validate_against(graph)?;
    let ranked = rank_schedulable_expansions_with_seed(
        graph,
        config.regime,
        config.generation,
        config.seed,
        &learned.estimates,
    )?;
    let queue = ReplayableExpansionQueue::seal(
        graph.content_sha256()?,
        learned.content_sha256()?,
        config.content_sha256()?,
        ranked
            .into_iter()
            .map(|entry| entry.expansion_sha256)
            .collect(),
    );
    queue.validate()?;
    Ok(queue)
}

pub fn lease_replayed_expansion(
    graph: &mut StateGraph,
    config: &ExpansionSchedulerConfig,
    learned: &GraphPrioritySnapshot,
    lease_sha256: Digest,
) -> Result<ReplayableExpansionQueue, SchedulerError> {
    if lease_sha256 == Digest::ZERO {
        return Err(SchedulerError::Invalid(
            "scheduler lease identity is missing",
        ));
    }
    let queue = replay_expansion_queue(graph, config, learned)?;
    if let Some(selected) = queue.selected_expansion_sha256 {
        graph.lease_action_expansion(
            selected,
            lease_sha256,
            config.generation,
            config
                .generation
                .checked_add(config.lease_generations)
                .ok_or(SchedulerError::Invalid(
                    "scheduler lease generation overflows",
                ))?,
        )?;
    }
    Ok(queue)
}

pub fn rank_schedulable_nodes(
    graph: &StateGraph,
    regime: SearchRegime,
    maximum_route_frames: u64,
    seed: u64,
    generation: u64,
) -> Result<Vec<ScheduledNode>, SchedulerError> {
    rank_schedulable_nodes_validated(
        graph.validated()?,
        regime,
        maximum_route_frames,
        seed,
        generation,
    )
}

pub(crate) fn rank_schedulable_nodes_validated(
    validated: ValidatedStateGraph<'_>,
    regime: SearchRegime,
    maximum_route_frames: u64,
    seed: u64,
    generation: u64,
) -> Result<Vec<ScheduledNode>, SchedulerError> {
    let graph = validated.graph();
    if regime == SearchRegime::Optimization && graph.best_terminal_path().is_none() {
        return Err(SchedulerError::Invalid(
            "optimization scheduling requires a terminal path",
        ));
    }
    let exact_terminal_returns = validated.exact_terminal_returns()?;
    let relaxed_root_ticks = graph.relaxed_root_ticks()?;
    let canonical_nodes = graph
        .nodes()
        .map(|node| Ok((node.id, graph.canonical_restoration_node(node.id)?)))
        .collect::<Result<BTreeMap<_, _>, SchedulerError>>()?;
    let expanded_references = graph
        .nodes()
        .filter(|node| node.id == graph.root() || !node.outgoing_expansions.is_empty())
        .map(|node| ReachabilityCell::from_state(&node.state))
        .collect::<Vec<_>>();
    let mut ranked = graph
        .nodes()
        .filter(|node| {
            node.id != graph.root()
                // Future-equivalence canonicalization may choose a faster
                // restoration from another route. That is useful for
                // discovery, but it must not erase an exact interior of an
                // authenticated terminal tape: the alternate route does not
                // itself carry that route-specific continuation.
                && (canonical_nodes.get(&node.id) == Some(&node.id)
                    || exact_terminal_returns.contains_key(&node.id))
                && node.restoration.executable
                && !node.terminal
                && node.restoration.route.tape_frames <= maximum_route_frames
        })
        .map(|node| {
            let registered_expansions = node.outgoing_expansions.len() as u64;
            let completed_expansions = node
                .outgoing_expansions
                .iter()
                .filter(|identity| {
                    graph.expansion(**identity).is_some_and(|expansion| {
                        matches!(
                            expansion.status,
                            crate::state_graph::ActionExpansionStatus::Completed { .. }
                        )
                    })
                })
                .count() as u64;
            let exact_terminal_ticks_to_go = exact_terminal_returns.get(&node.id).copied();
            ScheduledNode {
                node: node.id,
                root_ticks: relaxed_root_ticks
                    .get(&node.id)
                    .copied()
                    .unwrap_or(node.root_ticks),
                registered_expansions,
                completed_expansions,
                reachability_novelty: reachability_novelty(
                    &ReachabilityCell::from_state(&node.state),
                    &expanded_references,
                ),
                exact_terminal_ticks_to_go,
                tie_rank: node_tie_rank(seed, generation, node.id),
            }
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| compare_scheduled_node(regime, left, right));
    Ok(ranked)
}

pub fn rank_schedulable_expansions(
    graph: &StateGraph,
    regime: SearchRegime,
    current_generation: u64,
    learned: &BTreeMap<Digest, LearnedExpansionPriority>,
) -> Result<Vec<ScheduledExpansion>, SchedulerError> {
    rank_schedulable_expansions_validated(graph.validated()?, regime, current_generation, learned)
}

pub(crate) fn rank_schedulable_expansions_validated(
    graph: ValidatedStateGraph<'_>,
    regime: SearchRegime,
    current_generation: u64,
    learned: &BTreeMap<Digest, LearnedExpansionPriority>,
) -> Result<Vec<ScheduledExpansion>, SchedulerError> {
    rank_schedulable_expansions_with_seed_validated(graph, regime, current_generation, 0, learned)
}

fn rank_schedulable_expansions_with_seed(
    graph: &StateGraph,
    regime: SearchRegime,
    current_generation: u64,
    seed: u64,
    learned: &BTreeMap<Digest, LearnedExpansionPriority>,
) -> Result<Vec<ScheduledExpansion>, SchedulerError> {
    rank_schedulable_expansions_with_seed_validated(
        graph.validated()?,
        regime,
        current_generation,
        seed,
        learned,
    )
}

fn rank_schedulable_expansions_with_seed_validated(
    validated: ValidatedStateGraph<'_>,
    regime: SearchRegime,
    current_generation: u64,
    seed: u64,
    learned: &BTreeMap<Digest, LearnedExpansionPriority>,
) -> Result<Vec<ScheduledExpansion>, SchedulerError> {
    let graph = validated.graph();
    if regime == SearchRegime::Optimization && graph.best_terminal_path().is_none() {
        return Err(SchedulerError::Invalid(
            "optimization scheduling requires a terminal path",
        ));
    }
    for score in learned.values() {
        score.validate()?;
    }
    let relaxed_root_ticks = graph.relaxed_root_ticks()?;
    let exact_terminal_returns = validated.exact_terminal_returns()?;
    let mut ranked = graph
        .expansions()
        .filter(|expansion| {
            graph.expansion_is_schedulable(expansion.identity_sha256, current_generation)
        })
        .map(|expansion| {
            let node = graph
                .node(expansion.source)
                .ok_or(SchedulerError::Invalid("expansion source is absent"))?;
            let learned = learned
                .get(&expansion.identity_sha256)
                .copied()
                .unwrap_or_default();
            Ok(ScheduledExpansion {
                expansion_sha256: expansion.identity_sha256,
                source: expansion.source,
                source_root_ticks: relaxed_root_ticks
                    .get(&node.id)
                    .copied()
                    .unwrap_or(node.root_ticks),
                source_exact_terminal_ticks_to_go: exact_terminal_returns.get(&node.id).copied(),
                generalized_conditional_ticks_to_go: learned.conditional_ticks_to_go,
                uncertainty_millionths: learned.uncertainty_millionths,
                exploration_priority_rank: 0,
                learned,
            })
        })
        .collect::<Result<Vec<_>, SchedulerError>>()?;
    ranked.sort_by(|left, right| {
        compare_scheduled(regime, left, right).then_with(|| {
            expansion_tie_rank(seed, current_generation, left.expansion_sha256).cmp(
                &expansion_tie_rank(seed, current_generation, right.expansion_sha256),
            )
        })
    });
    for (rank, expansion) in ranked.iter_mut().enumerate() {
        expansion.exploration_priority_rank = rank as u64;
    }
    Ok(ranked)
}

#[allow(clippy::too_many_arguments)]
pub fn lease_next_expansion(
    graph: &mut StateGraph,
    regime: SearchRegime,
    current_generation: u64,
    learned: &BTreeMap<Digest, LearnedExpansionPriority>,
    lease_sha256: Digest,
    expires_at_generation: u64,
) -> Result<Option<ScheduledExpansion>, SchedulerError> {
    let Some(selected) = rank_schedulable_expansions(graph, regime, current_generation, learned)?
        .into_iter()
        .next()
    else {
        return Ok(None);
    };
    graph.lease_action_expansion(
        selected.expansion_sha256,
        lease_sha256,
        current_generation,
        expires_at_generation,
    )?;
    Ok(Some(selected))
}

fn compare_scheduled(
    regime: SearchRegime,
    left: &ScheduledExpansion,
    right: &ScheduledExpansion,
) -> Ordering {
    let discovery = || {
        left.learned
            .policy_rank
            .is_none()
            .cmp(&right.learned.policy_rank.is_none())
            .then_with(|| left.learned.policy_rank.cmp(&right.learned.policy_rank))
            .then_with(|| {
                left.learned
                    .completed_visits
                    .cmp(&right.learned.completed_visits)
            })
            .then_with(|| {
                right
                    .learned
                    .uncertainty_millionths
                    .cmp(&left.learned.uncertainty_millionths)
            })
            .then_with(|| {
                right
                    .learned
                    .prediction_error_millionths
                    .cmp(&left.learned.prediction_error_millionths)
            })
            .then_with(|| left.source_root_ticks.cmp(&right.source_root_ticks))
    };
    let ordering = match regime {
        SearchRegime::Discovery => discovery(),
        SearchRegime::Optimization => {
            let total_ticks = |entry: &ScheduledExpansion| {
                entry
                    .learned
                    .conditional_ticks_to_go
                    .map(|ticks| entry.source_root_ticks.saturating_add(ticks))
            };
            total_ticks(left)
                .is_none()
                .cmp(&total_ticks(right).is_none())
                .then_with(|| total_ticks(left).cmp(&total_ticks(right)))
                .then_with(|| {
                    right
                        .learned
                        .terminal_support_per_million
                        .cmp(&left.learned.terminal_support_per_million)
                })
                .then_with(discovery)
        }
    };
    ordering
}

fn compare_scheduled_node(
    regime: SearchRegime,
    left: &ScheduledNode,
    right: &ScheduledNode,
) -> Ordering {
    let coverage = || {
        left.completed_expansions
            .cmp(&right.completed_expansions)
            .then_with(|| left.registered_expansions.cmp(&right.registered_expansions))
    };
    let ordering = match regime {
        // Long options create many fresh deep boundaries on the same
        // wall-adjacent trajectory. Extend the graph's explored spatial
        // envelope before using raw route depth as a horizon tie-break.
        SearchRegime::Discovery => coverage()
            .then_with(|| right.reachability_novelty.cmp(&left.reachability_novelty))
            .then_with(|| right.root_ticks.cmp(&left.root_ticks)),
        SearchRegime::Optimization => left
            .exact_terminal_ticks_to_go
            .is_none()
            .cmp(&right.exact_terminal_ticks_to_go.is_none())
            .then_with(coverage)
            .then_with(|| left.root_ticks.cmp(&right.root_ticks)),
    };
    ordering
        .then_with(|| left.tie_rank.cmp(&right.tie_rank))
        .then_with(|| left.node.cmp(&right.node))
}

const REACHABILITY_POSITION_BIN_WORLD_UNITS: f64 = 64.0;

#[derive(Clone, Debug, Eq, PartialEq)]
struct ReachabilityCell {
    stage: String,
    room: i8,
    layer: Option<i8>,
    position_bin: [i64; 3],
}

impl ReachabilityCell {
    fn from_state(state: &dusklight_learning::fact_snapshot::FactSnapshot) -> Self {
        let position_bin = state.player.position_f32_bits.map(|bits| {
            (f64::from(f32::from_bits(bits)) / REACHABILITY_POSITION_BIN_WORLD_UNITS).floor() as i64
        });
        Self {
            stage: state.world.stage.clone(),
            room: state.world.room,
            layer: state.world.layer,
            position_bin,
        }
    }
}

fn reachability_novelty(candidate: &ReachabilityCell, references: &[ReachabilityCell]) -> u128 {
    references
        .iter()
        .map(|reference| reachability_distance(candidate, reference))
        .min()
        .unwrap_or(u128::MAX)
}

fn reachability_distance(left: &ReachabilityCell, right: &ReachabilityCell) -> u128 {
    let mut distance = 0_u128;
    if left.stage != right.stage {
        distance += 1_u128 << 127;
    }
    if left.room != right.room || left.layer != right.layer {
        distance += 1_u128 << 112;
    }
    for (left, right) in left.position_bin.iter().zip(right.position_bin) {
        distance = distance.saturating_add(u128::from(left.abs_diff(right)).pow(2));
    }
    distance
}

fn node_tie_rank(seed: u64, generation: u64, node: ExactStateId) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight-scheduled-node-tie/v1");
    hasher.update(seed.to_le_bytes());
    hasher.update(generation.to_le_bytes());
    hasher.update(node.route_checkpoint_sha256.0);
    hasher.update(node.state_sha256.0);
    Digest(hasher.finalize().into())
}

fn expansion_tie_rank(seed: u64, generation: u64, expansion_sha256: Digest) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight-scheduled-expansion-tie/v1");
    hasher.update(seed.to_le_bytes());
    hasher.update(generation.to_le_bytes());
    hasher.update(expansion_sha256.0);
    Digest(hasher.finalize().into())
}

fn hash_learned_priority(hasher: &mut Sha256, estimate: LearnedExpansionPriority) {
    hash_optional_u64(hasher, estimate.policy_rank);
    match estimate.terminal_support_per_million {
        Some(value) => {
            hasher.update([1]);
            hasher.update(value.to_le_bytes());
        }
        None => hasher.update([0]),
    }
    hash_optional_u64(hasher, estimate.conditional_ticks_to_go);
    hasher.update(estimate.uncertainty_millionths.to_le_bytes());
    hasher.update(estimate.prediction_error_millionths.to_le_bytes());
    hasher.update(estimate.completed_visits.to_le_bytes());
}

fn hash_optional_u64(hasher: &mut Sha256, value: Option<u64>) {
    match value {
        Some(value) => {
            hasher.update([1]);
            hasher.update(value.to_le_bytes());
        }
        None => hasher.update([0]),
    }
}

fn expansion_queue_sha256(
    graph_sha256: Digest,
    learner_priority_snapshot_sha256: Digest,
    scheduler_config_sha256: Digest,
    ranked_expansions: &[Digest],
) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(REPLAYABLE_EXPANSION_QUEUE_SCHEMA_V1.as_bytes());
    hasher.update(graph_sha256.0);
    hasher.update(learner_priority_snapshot_sha256.0);
    hasher.update(scheduler_config_sha256.0);
    hasher.update((ranked_expansions.len() as u64).to_le_bytes());
    for identity in ranked_expansions {
        hasher.update(identity.0);
    }
    Digest(hasher.finalize().into())
}

#[derive(Debug)]
pub enum SchedulerError {
    Invalid(&'static str),
    Graph(StateGraphError),
}

impl fmt::Display for SchedulerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) => write!(formatter, "invalid expansion schedule: {message}"),
            Self::Graph(error) => write!(formatter, "expansion schedule graph failed: {error}"),
        }
    }
}

impl Error for SchedulerError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Graph(error) => Some(error),
            Self::Invalid(_) => None,
        }
    }
}

impl From<StateGraphError> for SchedulerError {
    fn from(value: StateGraphError) -> Self {
        Self::Graph(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dusklight_automation_contracts::tape::{InputFrame, InputTape};
    use dusklight_control::option_execution::{
        OptionCondition, OptionEndReason, OptionExecution, OptionType, TapeRange,
    };
    use dusklight_evidence::native_episode_shard::NativeEpisodeShard;
    use dusklight_learning::fact_snapshot::{FactPhase, FactSnapshot, FactTerminalReason};
    use dusklight_learning::option_transition::OptionTransitionSample;
    use dusklight_learning::option_values::OptionActionDescriptor;
    use std::collections::BTreeMap;

    fn entry(
        identity: u8,
        root_ticks: u64,
        learned: LearnedExpansionPriority,
    ) -> ScheduledExpansion {
        ScheduledExpansion {
            expansion_sha256: Digest([identity; 32]),
            source: ExactStateId {
                route_checkpoint_sha256: Digest([identity; 32]),
                state_sha256: Digest([identity.saturating_add(1); 32]),
            },
            source_root_ticks: root_ticks,
            source_exact_terminal_ticks_to_go: None,
            generalized_conditional_ticks_to_go: learned.conditional_ticks_to_go,
            uncertainty_millionths: learned.uncertainty_millionths,
            exploration_priority_rank: 0,
            learned,
        }
    }

    fn node_entry(
        identity: u8,
        completed_expansions: u64,
        exact_terminal_ticks_to_go: Option<u64>,
    ) -> ScheduledNode {
        ScheduledNode {
            node: ExactStateId {
                route_checkpoint_sha256: Digest([identity; 32]),
                state_sha256: Digest([identity.saturating_add(1); 32]),
            },
            root_ticks: identity as u64,
            registered_expansions: completed_expansions,
            completed_expansions,
            reachability_novelty: 0,
            exact_terminal_ticks_to_go,
            tie_rank: Digest([identity; 32]),
        }
    }

    fn replay_graph() -> (StateGraph, Digest, Digest) {
        let shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../../tests/fixtures/automation/native_episode_v28.dseps"
        ))
        .unwrap();
        let state = FactSnapshot::from_native_learning(
            &shard.episodes[0].steps[0].pre_input,
            &[],
            None,
            Vec::new(),
        )
        .unwrap();
        let route = InputTape {
            frames: vec![InputFrame::default(); state.tape_frame as usize],
            ..InputTape::default()
        };
        let mut graph = StateGraph::new(
            crate::state_graph::StateGraphIdentity {
                execution_authority_sha256: Digest([1; 32]),
                future_equivalence_validator_sha256: Digest([1; 32]),
                feature_schema_sha256: Digest([2; 32]),
                objective_sha256: Digest([3; 32]),
                root_checkpoint_sha256: Digest([4; 32]),
            },
            state,
            route,
        )
        .unwrap();
        let first = graph
            .register_action_expansion(
                graph.root(),
                OptionActionDescriptor {
                    option_id: "move".into(),
                    option_type: OptionType::Move,
                    parameters: BTreeMap::new(),
                },
            )
            .unwrap();
        let second = graph
            .register_action_expansion(
                graph.root(),
                OptionActionDescriptor {
                    option_id: "turn".into(),
                    option_type: OptionType::Turn,
                    parameters: BTreeMap::new(),
                },
            )
            .unwrap();
        (graph, first, second)
    }

    fn terminal_replay_graph() -> (StateGraph, Digest) {
        let (mut graph, _, pending) = replay_graph();
        let before = graph.node(graph.root()).unwrap().state.as_ref().clone();
        let mut route = graph
            .route(graph.root().route_checkpoint_sha256)
            .unwrap()
            .clone();
        route.frames.extend(vec![InputFrame::default(); 8]);
        let execution = OptionExecution::capture(
            "move".into(),
            OptionType::Move,
            BTreeMap::new(),
            8,
            8,
            OptionCondition::DurationElapsed,
            Vec::new(),
            OptionEndReason::Completed,
            &route,
            TapeRange {
                start_frame: before.tape_frame,
                end_frame_exclusive: before.tape_frame + 8,
            },
        )
        .unwrap();
        let mut after = before.clone();
        after.phase = FactPhase::PreInput;
        after.boundary_index += 8;
        after.simulation_tick += 8;
        after.tape_frame += 8;
        after.recent_history.clear();
        after.recent_option = None;
        after.terminal.reached = Some(true);
        after.terminal.reason = FactTerminalReason::GoalReached;
        after.terminal.first_hit_tick = Some(after.simulation_tick);
        after.validate().unwrap();
        let next_checkpoint_sha256 = crate::state_graph::route_checkpoint_sha256(
            graph.identity.root_checkpoint_sha256,
            &route,
        )
        .unwrap();
        let mut transition = OptionTransitionSample::capture(
            graph.identity.feature_schema_sha256,
            graph.root().route_checkpoint_sha256,
            next_checkpoint_sha256,
            before,
            after,
            execution,
            &route,
            -8.0,
            true,
            |state| Ok::<_, &'static str>(vec![state.tape_frame as f32]),
        )
        .unwrap();
        transition.execution_authority_sha256 = graph.identity.execution_authority_sha256;
        transition.validate().unwrap();
        graph
            .admit_completed_expansion(
                transition,
                route,
                1,
                crate::state_graph::ExpansionEvidenceAuthority::Executable,
            )
            .unwrap();
        (graph, pending)
    }

    #[test]
    fn discovery_prefers_coverage_then_uncertainty_without_float_ordering() {
        let uncertain = entry(
            1,
            20,
            LearnedExpansionPriority {
                uncertainty_millionths: 900,
                ..Default::default()
            },
        );
        let visited = entry(
            2,
            10,
            LearnedExpansionPriority {
                uncertainty_millionths: 1_000,
                completed_visits: 1,
                ..Default::default()
            },
        );
        assert_eq!(
            compare_scheduled(SearchRegime::Discovery, &uncertain, &visited),
            Ordering::Less
        );
    }

    #[test]
    fn active_policy_rank_is_a_deterministic_fallback_before_uncertainty() {
        let primary = entry(
            1,
            20,
            LearnedExpansionPriority {
                policy_rank: Some(0),
                uncertainty_millionths: 10,
                ..Default::default()
            },
        );
        let secondary = entry(
            2,
            10,
            LearnedExpansionPriority {
                policy_rank: Some(1),
                uncertainty_millionths: 1_000,
                ..Default::default()
            },
        );
        assert_eq!(
            compare_scheduled(SearchRegime::Discovery, &primary, &secondary),
            Ordering::Less
        );
    }

    #[test]
    fn optimization_prefers_supported_lower_total_ticks() {
        let short = entry(
            1,
            20,
            LearnedExpansionPriority {
                terminal_support_per_million: Some(700_000),
                conditional_ticks_to_go: Some(30),
                ..Default::default()
            },
        );
        let long = entry(
            2,
            5,
            LearnedExpansionPriority {
                terminal_support_per_million: Some(900_000),
                conditional_ticks_to_go: Some(60),
                ..Default::default()
            },
        );
        assert_eq!(
            compare_scheduled(SearchRegime::Optimization, &short, &long),
            Ordering::Less
        );
    }

    #[test]
    fn scheduled_expansion_exposes_exact_generalized_uncertainty_and_queue_rank() {
        let (graph, pending) = terminal_replay_graph();
        let learned = BTreeMap::from([(
            pending,
            LearnedExpansionPriority {
                terminal_support_per_million: Some(750_000),
                conditional_ticks_to_go: Some(7),
                uncertainty_millionths: 125_000,
                ..Default::default()
            },
        )]);

        let scheduled =
            rank_schedulable_expansions(&graph, SearchRegime::Optimization, 2, &learned).unwrap();

        assert_eq!(scheduled.len(), 1);
        assert_eq!(scheduled[0].source_exact_terminal_ticks_to_go, Some(8));
        assert_eq!(scheduled[0].generalized_conditional_ticks_to_go, Some(7));
        assert_eq!(scheduled[0].uncertainty_millionths, 125_000);
        assert_eq!(scheduled[0].exploration_priority_rank, 0);
    }

    #[test]
    fn node_discovery_prefers_the_least_expanded_boundary() {
        let fresh = node_entry(2, 0, None);
        let visited = node_entry(1, 1, None);
        assert_eq!(
            compare_scheduled_node(SearchRegime::Discovery, &fresh, &visited),
            Ordering::Less
        );
    }

    #[test]
    fn node_discovery_prefers_deeper_reachability_when_coverage_is_equal() {
        let shallow = node_entry(1, 0, None);
        let deep = node_entry(9, 0, None);
        assert!(deep.root_ticks > shallow.root_ticks);
        assert_eq!(
            compare_scheduled_node(SearchRegime::Discovery, &deep, &shallow),
            Ordering::Less
        );
    }

    #[test]
    fn node_discovery_prefers_spatial_reachability_over_raw_depth() {
        let mut deep_wall_tail = node_entry(40, 0, None);
        deep_wall_tail.reachability_novelty = 1;
        let mut shallower_new_region = node_entry(20, 0, None);
        shallower_new_region.reachability_novelty = 25;

        assert_eq!(
            compare_scheduled_node(
                SearchRegime::Discovery,
                &shallower_new_region,
                &deep_wall_tail,
            ),
            Ordering::Less
        );
    }

    #[test]
    fn spatial_novelty_is_measured_from_expanded_graph_states() {
        let reference = ReachabilityCell {
            stage: "field".into(),
            room: 1,
            layer: Some(0),
            position_bin: [0, 0, 0],
        };
        let nearby = ReachabilityCell {
            position_bin: [1, 0, 0],
            ..reference.clone()
        };
        let unexplored = ReachabilityCell {
            position_bin: [3, 0, 4],
            ..reference.clone()
        };

        assert_eq!(
            reachability_novelty(&nearby, std::slice::from_ref(&reference)),
            1
        );
        assert_eq!(
            reachability_novelty(&unexplored, std::slice::from_ref(&reference)),
            25
        );
    }

    #[test]
    fn node_optimization_prefers_boundaries_on_the_exact_terminal_path() {
        let path = node_entry(2, 2, Some(20));
        let broad = node_entry(1, 0, None);
        assert_eq!(
            compare_scheduled_node(SearchRegime::Optimization, &path, &broad),
            Ordering::Less
        );
    }

    #[test]
    fn sealed_queue_replays_across_a_binary_graph_restart() {
        let (graph, first, second) = replay_graph();
        let learned = GraphPrioritySnapshot {
            schema: GRAPH_PRIORITY_SNAPSHOT_SCHEMA_V1.into(),
            graph_sha256: graph.content_sha256().unwrap(),
            learner_snapshot_sha256: Some(Digest([9; 32])),
            estimates: BTreeMap::from([
                (
                    first,
                    LearnedExpansionPriority {
                        policy_rank: Some(1),
                        ..Default::default()
                    },
                ),
                (
                    second,
                    LearnedExpansionPriority {
                        policy_rank: Some(0),
                        conditional_ticks_to_go: Some(9),
                        uncertainty_millionths: 42,
                        ..Default::default()
                    },
                ),
            ]),
        };
        let config = ExpansionSchedulerConfig {
            schema: EXPANSION_SCHEDULER_CONFIG_SCHEMA_V1.into(),
            regime: SearchRegime::Discovery,
            seed: 17,
            generation: 4,
            lease_generations: 3,
        };

        let before = replay_expansion_queue(&graph, &config, &learned).unwrap();
        let restored = StateGraph::decode(&graph.encode().unwrap()).unwrap();
        let after = replay_expansion_queue(&restored, &config, &learned).unwrap();
        let inspected = rank_schedulable_expansions_with_seed(
            &graph,
            config.regime,
            config.generation,
            config.seed,
            &learned.estimates,
        )
        .unwrap();

        assert_eq!(before, after);
        assert_eq!(before.selected_expansion_sha256, Some(second));
        assert_eq!(before.ranked_expansions, vec![second, first]);
        assert_eq!(inspected[0].exploration_priority_rank, 0);
        assert_eq!(inspected[1].exploration_priority_rank, 1);
        assert_eq!(inspected[0].source_exact_terminal_ticks_to_go, None);
        assert_eq!(inspected[0].generalized_conditional_ticks_to_go, Some(9));
        assert_eq!(inspected[0].uncertainty_millionths, 42);
        before.validate().unwrap();
    }

    #[test]
    fn leasing_consumes_the_replayed_graph_owned_expansion() {
        let (mut graph, first, second) = replay_graph();
        let learned = GraphPrioritySnapshot {
            schema: GRAPH_PRIORITY_SNAPSHOT_SCHEMA_V1.into(),
            graph_sha256: graph.content_sha256().unwrap(),
            learner_snapshot_sha256: Some(Digest([8; 32])),
            estimates: BTreeMap::from([
                (
                    first,
                    LearnedExpansionPriority {
                        policy_rank: Some(1),
                        ..Default::default()
                    },
                ),
                (
                    second,
                    LearnedExpansionPriority {
                        policy_rank: Some(0),
                        ..Default::default()
                    },
                ),
            ]),
        };
        let config = ExpansionSchedulerConfig {
            schema: EXPANSION_SCHEDULER_CONFIG_SCHEMA_V1.into(),
            regime: SearchRegime::Discovery,
            seed: 23,
            generation: 10,
            lease_generations: 2,
        };

        let queue =
            lease_replayed_expansion(&mut graph, &config, &learned, Digest([7; 32])).unwrap();

        assert_eq!(queue.selected_expansion_sha256, Some(second));
        assert!(!graph.expansion_is_schedulable(second, 10));
        assert!(graph.expansion_is_schedulable(second, 12));
        assert!(replay_expansion_queue(&graph, &config, &learned).is_err());
    }
}
