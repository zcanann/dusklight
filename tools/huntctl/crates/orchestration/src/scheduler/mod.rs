//! Deterministic scheduling over graph-owned expansion lifecycle state.
//!
//! The scheduler ranks work but never owns it. Leasing mutates the state graph,
//! which supplies virtual loss to every worker sharing that graph.

use crate::state_graph::{ExactStateId, StateGraph, StateGraphError};
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
    pub learned: LearnedExpansionPriority,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScheduledNode {
    pub node: ExactStateId,
    pub root_ticks: u64,
    pub registered_expansions: u64,
    pub completed_expansions: u64,
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
    graph.validate()?;
    if regime == SearchRegime::Optimization && graph.best_terminal_path().is_none() {
        return Err(SchedulerError::Invalid(
            "optimization scheduling requires a terminal path",
        ));
    }
    let best_terminal_route = graph
        .best_terminal_path()
        .and_then(|path| graph.route(path.route_checkpoint_sha256));
    let best_terminal_ticks = graph
        .best_terminal_path()
        .map(|path| path.root_to_terminal_ticks);
    let mut ranked = graph
        .nodes()
        .filter(|node| {
            node.id != graph.root()
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
            let route = graph.route(node.id.route_checkpoint_sha256);
            let exact_terminal_ticks_to_go = match (route, best_terminal_route, best_terminal_ticks)
            {
                (Some(route), Some(best), Some(total))
                    if same_tape_origin(route, best)
                        && best.frames.starts_with(&route.frames)
                        && total >= node.root_ticks =>
                {
                    Some(total - node.root_ticks)
                }
                _ => None,
            };
            ScheduledNode {
                node: node.id,
                root_ticks: node.root_ticks,
                registered_expansions,
                completed_expansions,
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
    rank_schedulable_expansions_with_seed(graph, regime, current_generation, 0, learned)
}

fn rank_schedulable_expansions_with_seed(
    graph: &StateGraph,
    regime: SearchRegime,
    current_generation: u64,
    seed: u64,
    learned: &BTreeMap<Digest, LearnedExpansionPriority>,
) -> Result<Vec<ScheduledExpansion>, SchedulerError> {
    graph.validate()?;
    if regime == SearchRegime::Optimization && graph.best_terminal_path().is_none() {
        return Err(SchedulerError::Invalid(
            "optimization scheduling requires a terminal path",
        ));
    }
    for score in learned.values() {
        score.validate()?;
    }
    let mut ranked = graph
        .expansions()
        .filter(|expansion| {
            graph.expansion_is_schedulable(expansion.identity_sha256, current_generation)
        })
        .map(|expansion| {
            let node = graph
                .node(expansion.source)
                .ok_or(SchedulerError::Invalid("expansion source is absent"))?;
            Ok(ScheduledExpansion {
                expansion_sha256: expansion.identity_sha256,
                source: expansion.source,
                source_root_ticks: node.root_ticks,
                learned: learned
                    .get(&expansion.identity_sha256)
                    .copied()
                    .unwrap_or_default(),
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
            .then_with(|| left.root_ticks.cmp(&right.root_ticks))
    };
    let ordering = match regime {
        SearchRegime::Discovery => coverage(),
        SearchRegime::Optimization => left
            .exact_terminal_ticks_to_go
            .is_none()
            .cmp(&right.exact_terminal_ticks_to_go.is_none())
            .then_with(coverage),
    };
    ordering
        .then_with(|| left.tie_rank.cmp(&right.tie_rank))
        .then_with(|| left.node.cmp(&right.node))
}

fn same_tape_origin(
    left: &dusklight_automation_contracts::tape::InputTape,
    right: &dusklight_automation_contracts::tape::InputTape,
) -> bool {
    left.boot == right.boot
        && left.tick_rate_numerator == right.tick_rate_numerator
        && left.tick_rate_denominator == right.tick_rate_denominator
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
    use dusklight_control::option_execution::OptionType;
    use dusklight_evidence::native_episode_shard::NativeEpisodeShard;
    use dusklight_learning::fact_snapshot::FactSnapshot;
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
    fn node_discovery_prefers_the_least_expanded_boundary() {
        let fresh = node_entry(2, 0, None);
        let visited = node_entry(1, 1, None);
        assert_eq!(
            compare_scheduled_node(SearchRegime::Discovery, &fresh, &visited),
            Ordering::Less
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

        assert_eq!(before, after);
        assert_eq!(before.selected_expansion_sha256, Some(second));
        assert_eq!(before.ranked_expansions, vec![second, first]);
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
