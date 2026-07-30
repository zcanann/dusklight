use super::scratch_discovery::route_report_sha256;
use super::*;
use crate::state_graph::{
    ActionExpansionStatus, ExactStateId, ExpansionEvidenceAuthority, StateGraph,
};
use crate::tactic_q_campaign::{TacticQCampaign, TacticScheduledExpansionEvidence};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

pub const NATIVE_TACTIC_POST_TERMINAL_CONTROL_SCHEMA_V1: &str =
    "dusklight-native-tactic-post-terminal-control/v1";
pub const NATIVE_TACTIC_POST_TERMINAL_CONTROL_SCHEMA_V2: &str =
    "dusklight-native-tactic-post-terminal-control/v2";
const CONTROL_SEED: u64 = 0x5054_434f_4e54_0001;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeTacticPostTerminalControl {
    LearnedTotalTicks,
    LeastVisited,
    RandomValid,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticPostTerminalRanking {
    pub control: NativeTacticPostTerminalControl,
    pub supported_candidates: usize,
    pub selected_expansion_sha256: Digest,
    pub selected_total_ticks: Option<u64>,
    /// One-based rank of the first candidate with the best observed exact
    /// outcome. Unknown candidates ahead of it still consume an evaluation.
    pub evaluations_to_best_observed: Option<usize>,
    pub selected_observed_regret_ticks: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticPostTerminalDecisionControl {
    pub decision_index: u64,
    pub learner_model_sha256: Digest,
    pub queue_sha256: Digest,
    pub ranked_source_queue: Vec<TacticScheduledExpansionEvidence>,
    pub exact_total_ticks_by_expansion: BTreeMap<Digest, u64>,
    pub candidate_count: usize,
    pub exact_outcome_candidates: usize,
    /// True only when every queued action has an authenticated continuation to
    /// terminal in the final graph. Without this, exhaustive-local is not an
    /// oracle and no oracle-recovery claim is allowed.
    pub exhaustive_surface_complete: bool,
    pub best_observed_total_ticks: Option<u64>,
    pub exhaustive_local_evaluations: Option<usize>,
    pub rankings: Vec<NativeTacticPostTerminalRanking>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticPostTerminalSeedControl {
    pub seed: u64,
    pub state_graph_sha256: Digest,
    /// Every executable, non-root, nonterminal exact node on any
    /// authenticated terminal tape in the final graph.
    pub supported_interior_nodes: Vec<ExactStateId>,
    /// Supported interiors that actually sourced at least one evaluated graph
    /// lease in the retained decision trace.
    pub leased_supported_interior_nodes: Vec<ExactStateId>,
    pub unleased_supported_interior_nodes: Vec<ExactStateId>,
    pub complete_supported_interior_coverage: bool,
    pub optimization_decisions: usize,
    pub comparable_decisions: usize,
    pub exhaustive_complete_decisions: usize,
    pub learned_top_wins: usize,
    pub visit_top_wins: usize,
    pub random_top_wins: usize,
    pub learned_oracle_recoveries_with_fewer_evaluations: usize,
    pub decisions: Vec<NativeTacticPostTerminalDecisionControl>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticPostTerminalControlReport {
    pub schema: String,
    pub content_sha256: Digest,
    pub route_report_sha256: Digest,
    pub execution_plan_sha256: Digest,
    pub objective_sha256: Digest,
    pub control_seed: u64,
    pub supported_interior_nodes: usize,
    pub leased_supported_interior_nodes: usize,
    pub seeds_with_complete_supported_interior_coverage: usize,
    pub optimization_decisions: usize,
    pub comparable_decisions: usize,
    pub exhaustive_complete_decisions: usize,
    pub learned_top_wins: usize,
    pub visit_top_wins: usize,
    pub random_top_wins: usize,
    pub learned_oracle_recoveries_with_fewer_evaluations: usize,
    pub seeds: Vec<NativeTacticPostTerminalSeedControl>,
}

impl NativeTacticPostTerminalControlReport {
    pub fn build(
        repository_root: &Path,
        route: &NativeTacticRouteReport,
    ) -> Result<Self, NativeTacticRouteRunError> {
        let repository_root = repository_root.canonicalize().map_err(route_error)?;
        let mut seeds = Vec::new();
        for seed in &route.seeds {
            let checkpoint_path =
                confined_checkpoint(&repository_root, Path::new(&seed.final_checkpoint))?;
            let checkpoint =
                TacticQCampaign::read_checkpoint_payload(&checkpoint_path).map_err(route_error)?;
            let graph_sha256 = checkpoint
                .state_graph
                .content_sha256()
                .map_err(route_error)?;
            if graph_sha256 != seed.state_graph_sha256 {
                return Err(route_message(
                    "post-terminal control checkpoint graph identity differs",
                ));
            }
            seeds.push(seed_control(seed, &checkpoint.state_graph)?);
        }
        seeds.sort_by_key(|seed| seed.seed);
        let mut report = Self {
            schema: NATIVE_TACTIC_POST_TERMINAL_CONTROL_SCHEMA_V2.into(),
            content_sha256: Digest::ZERO,
            route_report_sha256: route_report_sha256(route)?,
            execution_plan_sha256: route.execution_plan_sha256,
            objective_sha256: route.objective_sha256,
            control_seed: CONTROL_SEED,
            supported_interior_nodes: seeds
                .iter()
                .map(|seed| seed.supported_interior_nodes.len())
                .sum(),
            leased_supported_interior_nodes: seeds
                .iter()
                .map(|seed| seed.leased_supported_interior_nodes.len())
                .sum(),
            seeds_with_complete_supported_interior_coverage: seeds
                .iter()
                .filter(|seed| seed.complete_supported_interior_coverage)
                .count(),
            optimization_decisions: seeds.iter().map(|seed| seed.optimization_decisions).sum(),
            comparable_decisions: seeds.iter().map(|seed| seed.comparable_decisions).sum(),
            exhaustive_complete_decisions: seeds
                .iter()
                .map(|seed| seed.exhaustive_complete_decisions)
                .sum(),
            learned_top_wins: seeds.iter().map(|seed| seed.learned_top_wins).sum(),
            visit_top_wins: seeds.iter().map(|seed| seed.visit_top_wins).sum(),
            random_top_wins: seeds.iter().map(|seed| seed.random_top_wins).sum(),
            learned_oracle_recoveries_with_fewer_evaluations: seeds
                .iter()
                .map(|seed| seed.learned_oracle_recoveries_with_fewer_evaluations)
                .sum(),
            seeds,
        };
        report.content_sha256 = report.compute_content_sha256()?;
        report.validate()?;
        Ok(report)
    }

    pub fn validate(&self) -> Result<(), NativeTacticRouteRunError> {
        let unique_seeds = self
            .seeds
            .iter()
            .map(|seed| seed.seed)
            .collect::<BTreeSet<_>>();
        if self.schema != NATIVE_TACTIC_POST_TERMINAL_CONTROL_SCHEMA_V2
            || self.content_sha256 == Digest::ZERO
            || self.route_report_sha256 == Digest::ZERO
            || self.execution_plan_sha256 == Digest::ZERO
            || self.objective_sha256 == Digest::ZERO
            || self.control_seed != CONTROL_SEED
            || self.seeds.is_empty()
            || unique_seeds.len() != self.seeds.len()
            || !self
                .seeds
                .windows(2)
                .all(|pair| pair[0].seed < pair[1].seed)
            || self.seeds.iter().any(|seed| !seed_control_is_valid(seed))
            || self.supported_interior_nodes
                != self
                    .seeds
                    .iter()
                    .map(|seed| seed.supported_interior_nodes.len())
                    .sum::<usize>()
            || self.leased_supported_interior_nodes
                != self
                    .seeds
                    .iter()
                    .map(|seed| seed.leased_supported_interior_nodes.len())
                    .sum::<usize>()
            || self.seeds_with_complete_supported_interior_coverage
                != self
                    .seeds
                    .iter()
                    .filter(|seed| seed.complete_supported_interior_coverage)
                    .count()
            || self.optimization_decisions
                != self
                    .seeds
                    .iter()
                    .map(|seed| seed.optimization_decisions)
                    .sum::<usize>()
            || self.comparable_decisions
                != self
                    .seeds
                    .iter()
                    .map(|seed| seed.comparable_decisions)
                    .sum::<usize>()
            || self.exhaustive_complete_decisions
                != self
                    .seeds
                    .iter()
                    .map(|seed| seed.exhaustive_complete_decisions)
                    .sum::<usize>()
            || self.learned_top_wins
                != self
                    .seeds
                    .iter()
                    .map(|seed| seed.learned_top_wins)
                    .sum::<usize>()
            || self.visit_top_wins
                != self
                    .seeds
                    .iter()
                    .map(|seed| seed.visit_top_wins)
                    .sum::<usize>()
            || self.random_top_wins
                != self
                    .seeds
                    .iter()
                    .map(|seed| seed.random_top_wins)
                    .sum::<usize>()
            || self.learned_oracle_recoveries_with_fewer_evaluations
                != self
                    .seeds
                    .iter()
                    .map(|seed| seed.learned_oracle_recoveries_with_fewer_evaluations)
                    .sum::<usize>()
            || self.content_sha256 != self.compute_content_sha256()?
        {
            return Err(route_message(
                "post-terminal tactic control report is invalid or detached",
            ));
        }
        Ok(())
    }

    pub fn to_pretty_json(&self) -> Result<Vec<u8>, NativeTacticRouteRunError> {
        self.validate()?;
        let mut bytes = serde_json::to_vec_pretty(self).map_err(route_error)?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    fn compute_content_sha256(&self) -> Result<Digest, NativeTacticRouteRunError> {
        let payload = serde_json::to_vec(&(
            &self.schema,
            self.route_report_sha256,
            self.execution_plan_sha256,
            self.objective_sha256,
            self.control_seed,
            self.supported_interior_nodes,
            self.leased_supported_interior_nodes,
            self.seeds_with_complete_supported_interior_coverage,
            self.optimization_decisions,
            self.comparable_decisions,
            self.exhaustive_complete_decisions,
            self.learned_top_wins,
            self.visit_top_wins,
            self.random_top_wins,
            self.learned_oracle_recoveries_with_fewer_evaluations,
            &self.seeds,
        ))
        .map_err(route_error)?;
        let mut hasher = Sha256::new();
        hasher.update(NATIVE_TACTIC_POST_TERMINAL_CONTROL_SCHEMA_V2.as_bytes());
        hasher.update((payload.len() as u64).to_le_bytes());
        hasher.update(payload);
        Ok(Digest(hasher.finalize().into()))
    }
}

fn seed_control(
    seed: &NativeTacticSeedResult,
    graph: &StateGraph,
) -> Result<NativeTacticPostTerminalSeedControl, NativeTacticRouteRunError> {
    let supported_interior_nodes = graph
        .exact_terminal_returns()
        .map_err(route_error)?
        .into_keys()
        .filter(|node| {
            *node != graph.root()
                && graph
                    .node(*node)
                    .is_some_and(|node| !node.terminal && node.restoration.executable)
        })
        .collect::<BTreeSet<_>>();
    let mut leased_supported_interior_nodes = BTreeSet::new();
    for trace in &seed.trace {
        let Some(decision) = &trace.scheduler_decision else {
            continue;
        };
        for expansion_sha256 in &decision.evaluated_expansion_sha256 {
            let expansion = graph.expansion(*expansion_sha256).ok_or_else(|| {
                route_message("post-terminal trace expansion is absent from its final graph")
            })?;
            if supported_interior_nodes.contains(&expansion.source) {
                leased_supported_interior_nodes.insert(expansion.source);
            }
        }
    }
    let unleased_supported_interior_nodes = supported_interior_nodes
        .difference(&leased_supported_interior_nodes)
        .copied()
        .collect::<Vec<_>>();
    let complete_supported_interior_coverage =
        !supported_interior_nodes.is_empty() && unleased_supported_interior_nodes.is_empty();
    let outcomes = exact_expansion_total_ticks(graph)?;
    let mut decisions = seed
        .trace
        .iter()
        .filter_map(|trace| {
            trace
                .scheduler_decision
                .as_ref()
                .map(|decision| (trace, decision))
        })
        .filter(|(_, decision)| {
            decision.regime == crate::scheduler::SearchRegime::Optimization
                && decision
                    .ranked_source_queue
                    .iter()
                    .all(|candidate| candidate.source_exact_terminal_ticks_to_go.is_some())
        })
        .map(|(trace, decision)| {
            decision.validate()?;
            evaluate_decision(seed.seed, trace.decision_index, decision, &outcomes)
        })
        .collect::<Result<Vec<_>, TacticQCampaignError>>()
        .map_err(route_error)?;
    decisions.sort_by_key(|decision| decision.decision_index);
    let comparable = decisions
        .iter()
        .filter(|decision| decision.exact_outcome_candidates >= 2)
        .collect::<Vec<_>>();
    let exhaustive = decisions
        .iter()
        .filter(|decision| decision.exhaustive_surface_complete && decision.candidate_count >= 2)
        .collect::<Vec<_>>();
    Ok(NativeTacticPostTerminalSeedControl {
        seed: seed.seed,
        state_graph_sha256: seed.state_graph_sha256,
        supported_interior_nodes: supported_interior_nodes.into_iter().collect(),
        leased_supported_interior_nodes: leased_supported_interior_nodes.into_iter().collect(),
        unleased_supported_interior_nodes,
        complete_supported_interior_coverage,
        optimization_decisions: decisions.len(),
        comparable_decisions: comparable.len(),
        exhaustive_complete_decisions: exhaustive.len(),
        learned_top_wins: top_wins(
            &comparable,
            NativeTacticPostTerminalControl::LearnedTotalTicks,
        ),
        visit_top_wins: top_wins(&comparable, NativeTacticPostTerminalControl::LeastVisited),
        random_top_wins: top_wins(&comparable, NativeTacticPostTerminalControl::RandomValid),
        learned_oracle_recoveries_with_fewer_evaluations: exhaustive
            .iter()
            .filter(|decision| {
                ranking(decision, NativeTacticPostTerminalControl::LearnedTotalTicks)
                    .evaluations_to_best_observed
                    .is_some_and(|evaluations| evaluations < decision.candidate_count)
            })
            .count(),
        decisions,
    })
}

fn exact_expansion_total_ticks(
    graph: &StateGraph,
) -> Result<BTreeMap<Digest, u64>, NativeTacticRouteRunError> {
    let returns = graph.exact_terminal_returns().map_err(route_error)?;
    let mut outcomes = BTreeMap::new();
    for expansion in graph.expansions() {
        if !matches!(
            expansion.status,
            ActionExpansionStatus::Completed {
                authority: ExpansionEvidenceAuthority::Executable,
                ..
            }
        ) {
            continue;
        }
        let Some(target) = expansion.target else {
            continue;
        };
        let Some(remaining) = returns.get(&target) else {
            continue;
        };
        let target_node = graph
            .node(target)
            .ok_or_else(|| route_message("completed expansion target is absent"))?;
        outcomes.insert(
            expansion.identity_sha256,
            target_node.root_ticks.saturating_add(*remaining),
        );
    }
    Ok(outcomes)
}

fn evaluate_decision(
    seed: u64,
    decision_index: u64,
    decision: &TacticSchedulerDecisionTrace,
    outcomes: &BTreeMap<Digest, u64>,
) -> Result<NativeTacticPostTerminalDecisionControl, TacticQCampaignError> {
    let relevant_outcomes = decision
        .ranked_source_queue
        .iter()
        .filter_map(|candidate| {
            outcomes
                .get(&candidate.expansion_sha256)
                .map(|ticks| (candidate.expansion_sha256, *ticks))
        })
        .collect::<BTreeMap<_, _>>();
    let best_observed = relevant_outcomes.values().copied().min();
    let rankings = [
        NativeTacticPostTerminalControl::LearnedTotalTicks,
        NativeTacticPostTerminalControl::LeastVisited,
        NativeTacticPostTerminalControl::RandomValid,
    ]
    .into_iter()
    .map(|control| {
        evaluate_ranking(
            seed,
            decision_index,
            &decision.ranked_source_queue,
            control,
            &relevant_outcomes,
            best_observed,
        )
    })
    .collect();
    let exact_outcome_candidates = relevant_outcomes.len();
    let complete = exact_outcome_candidates == decision.ranked_source_queue.len();
    let exhaustive_local_evaluations =
        (complete && exact_outcome_candidates >= 2).then_some(exact_outcome_candidates);
    Ok(NativeTacticPostTerminalDecisionControl {
        decision_index,
        learner_model_sha256: decision.learner_model_sha256,
        queue_sha256: decision.queue_sha256,
        ranked_source_queue: decision.ranked_source_queue.clone(),
        exact_total_ticks_by_expansion: relevant_outcomes,
        candidate_count: decision.ranked_source_queue.len(),
        exact_outcome_candidates,
        exhaustive_surface_complete: complete,
        best_observed_total_ticks: best_observed,
        exhaustive_local_evaluations,
        rankings,
    })
}

fn evaluate_ranking(
    seed: u64,
    decision_index: u64,
    queue: &[TacticScheduledExpansionEvidence],
    control: NativeTacticPostTerminalControl,
    outcomes: &BTreeMap<Digest, u64>,
    best_observed: Option<u64>,
) -> NativeTacticPostTerminalRanking {
    let mut candidates = queue.iter().collect::<Vec<_>>();
    candidates.sort_by(|left, right| compare_control(seed, decision_index, control, left, right));
    let selected = candidates[0];
    let selected_ticks = outcomes.get(&selected.expansion_sha256).copied();
    let evaluations_to_best = best_observed.and_then(|best| {
        candidates
            .iter()
            .position(|candidate| outcomes.get(&candidate.expansion_sha256) == Some(&best))
            .map(|index| index + 1)
    });
    NativeTacticPostTerminalRanking {
        control,
        supported_candidates: if control == NativeTacticPostTerminalControl::LearnedTotalTicks {
            candidates
                .iter()
                .filter(|candidate| candidate.generalized_conditional_ticks_to_go.is_some())
                .count()
        } else {
            candidates.len()
        },
        selected_expansion_sha256: selected.expansion_sha256,
        selected_total_ticks: selected_ticks,
        evaluations_to_best_observed: evaluations_to_best,
        selected_observed_regret_ticks: selected_ticks
            .zip(best_observed)
            .map(|(selected, best)| selected.saturating_sub(best)),
    }
}

fn compare_control(
    seed: u64,
    decision_index: u64,
    control: NativeTacticPostTerminalControl,
    left: &TacticScheduledExpansionEvidence,
    right: &TacticScheduledExpansionEvidence,
) -> Ordering {
    match control {
        NativeTacticPostTerminalControl::LearnedTotalTicks => {
            let total = |candidate: &TacticScheduledExpansionEvidence| {
                candidate
                    .generalized_conditional_ticks_to_go
                    .map(|ticks| candidate.source_root_ticks.saturating_add(ticks))
            };
            total(left)
                .is_none()
                .cmp(&total(right).is_none())
                .then_with(|| total(left).cmp(&total(right)))
        }
        NativeTacticPostTerminalControl::LeastVisited => {
            left.completed_visits.cmp(&right.completed_visits)
        }
        NativeTacticPostTerminalControl::RandomValid => random_key(
            seed,
            decision_index,
            left.expansion_sha256,
        )
        .cmp(&random_key(seed, decision_index, right.expansion_sha256)),
    }
    .then_with(|| left.expansion_sha256.cmp(&right.expansion_sha256))
}

fn random_key(seed: u64, decision_index: u64, expansion: Digest) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight-post-terminal-random-valid/v1");
    hasher.update(CONTROL_SEED.to_le_bytes());
    hasher.update(seed.to_le_bytes());
    hasher.update(decision_index.to_le_bytes());
    hasher.update(expansion.0);
    Digest(hasher.finalize().into())
}

fn top_wins(
    decisions: &[&NativeTacticPostTerminalDecisionControl],
    control: NativeTacticPostTerminalControl,
) -> usize {
    decisions
        .iter()
        .filter(|decision| {
            ranking(decision, control).selected_total_ticks == decision.best_observed_total_ticks
        })
        .count()
}

fn ranking(
    decision: &NativeTacticPostTerminalDecisionControl,
    control: NativeTacticPostTerminalControl,
) -> &NativeTacticPostTerminalRanking {
    decision
        .rankings
        .iter()
        .find(|ranking| ranking.control == control)
        .expect("validated decision has every control")
}

fn seed_control_is_valid(seed: &NativeTacticPostTerminalSeedControl) -> bool {
    let supported_interior_nodes = seed
        .supported_interior_nodes
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let leased_supported_interior_nodes = seed
        .leased_supported_interior_nodes
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let unleased_supported_interior_nodes = seed
        .unleased_supported_interior_nodes
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let expected_unleased = supported_interior_nodes
        .difference(&leased_supported_interior_nodes)
        .copied()
        .collect::<BTreeSet<_>>();
    let comparable = seed
        .decisions
        .iter()
        .filter(|decision| decision.exact_outcome_candidates >= 2)
        .collect::<Vec<_>>();
    let exhaustive = seed
        .decisions
        .iter()
        .filter(|decision| decision.exhaustive_surface_complete && decision.candidate_count >= 2)
        .collect::<Vec<_>>();
    seed.state_graph_sha256 != Digest::ZERO
        && supported_interior_nodes.len() == seed.supported_interior_nodes.len()
        && leased_supported_interior_nodes.len() == seed.leased_supported_interior_nodes.len()
        && unleased_supported_interior_nodes.len() == seed.unleased_supported_interior_nodes.len()
        && leased_supported_interior_nodes.is_subset(&supported_interior_nodes)
        && unleased_supported_interior_nodes == expected_unleased
        && seed.complete_supported_interior_coverage
            == (!seed.supported_interior_nodes.is_empty()
                && seed.unleased_supported_interior_nodes.is_empty())
        && seed
            .supported_interior_nodes
            .windows(2)
            .all(|pair| pair[0] < pair[1])
        && seed
            .leased_supported_interior_nodes
            .windows(2)
            .all(|pair| pair[0] < pair[1])
        && seed
            .unleased_supported_interior_nodes
            .windows(2)
            .all(|pair| pair[0] < pair[1])
        && seed.optimization_decisions == seed.decisions.len()
        && seed.comparable_decisions == comparable.len()
        && seed.exhaustive_complete_decisions == exhaustive.len()
        && seed.learned_top_wins
            == top_wins(
                &comparable,
                NativeTacticPostTerminalControl::LearnedTotalTicks,
            )
        && seed.visit_top_wins
            == top_wins(&comparable, NativeTacticPostTerminalControl::LeastVisited)
        && seed.random_top_wins
            == top_wins(&comparable, NativeTacticPostTerminalControl::RandomValid)
        && seed.learned_oracle_recoveries_with_fewer_evaluations
            == exhaustive
                .iter()
                .filter(|decision| {
                    ranking(decision, NativeTacticPostTerminalControl::LearnedTotalTicks)
                        .evaluations_to_best_observed
                        .is_some_and(|evaluations| evaluations < decision.candidate_count)
                })
                .count()
        && seed
            .decisions
            .windows(2)
            .all(|pair| pair[0].decision_index < pair[1].decision_index)
        && seed
            .decisions
            .iter()
            .all(|decision| decision_control_is_valid(seed.seed, decision))
}

fn decision_control_is_valid(
    seed: u64,
    decision: &NativeTacticPostTerminalDecisionControl,
) -> bool {
    let controls = decision
        .rankings
        .iter()
        .map(|ranking| ranking.control)
        .collect::<BTreeSet<_>>();
    let queue_identities = decision
        .ranked_source_queue
        .iter()
        .map(|candidate| candidate.expansion_sha256)
        .collect::<BTreeSet<_>>();
    let best_observed = decision
        .exact_total_ticks_by_expansion
        .values()
        .copied()
        .min();
    let expected_rankings = [
        NativeTacticPostTerminalControl::LearnedTotalTicks,
        NativeTacticPostTerminalControl::LeastVisited,
        NativeTacticPostTerminalControl::RandomValid,
    ]
    .into_iter()
    .map(|control| {
        evaluate_ranking(
            seed,
            decision.decision_index,
            &decision.ranked_source_queue,
            control,
            &decision.exact_total_ticks_by_expansion,
            best_observed,
        )
    })
    .collect::<Vec<_>>();
    decision.learner_model_sha256 != Digest::ZERO
        && decision.queue_sha256 != Digest::ZERO
        && decision.candidate_count == decision.ranked_source_queue.len()
        && decision.candidate_count > 0
        && queue_identities.len() == decision.candidate_count
        && decision
            .exact_total_ticks_by_expansion
            .keys()
            .all(|identity| queue_identities.contains(identity))
        && decision.exact_outcome_candidates == decision.exact_total_ticks_by_expansion.len()
        && decision.exact_outcome_candidates <= decision.candidate_count
        && decision.exhaustive_surface_complete
            == (decision.exact_outcome_candidates == decision.candidate_count)
        && decision.best_observed_total_ticks.is_some() == (decision.exact_outcome_candidates > 0)
        && decision.exhaustive_local_evaluations
            == (decision.exhaustive_surface_complete && decision.candidate_count >= 2)
                .then_some(decision.candidate_count)
        && controls
            == BTreeSet::from([
                NativeTacticPostTerminalControl::LearnedTotalTicks,
                NativeTacticPostTerminalControl::LeastVisited,
                NativeTacticPostTerminalControl::RandomValid,
            ])
        && decision.rankings.len() == controls.len()
        && decision.rankings == expected_rankings
        && decision.rankings.iter().all(|ranking| {
            ranking.selected_expansion_sha256 != Digest::ZERO
                && ranking.supported_candidates <= decision.candidate_count
                && ranking
                    .evaluations_to_best_observed
                    .is_none_or(|value| (1..=decision.candidate_count).contains(&value))
                && ranking.selected_observed_regret_ticks.is_some()
                    == ranking.selected_total_ticks.is_some()
        })
}

fn confined_checkpoint(
    repository_root: &Path,
    declared: &Path,
) -> Result<PathBuf, NativeTacticRouteRunError> {
    let candidate = if declared.is_absolute() {
        declared.to_path_buf()
    } else {
        repository_root.join(declared)
    };
    let resolved = candidate.canonicalize().map_err(route_error)?;
    if !resolved.starts_with(repository_root) || !resolved.is_file() {
        return Err(route_message(
            "post-terminal checkpoint is outside the repository or absent",
        ));
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exact_state(route: u8, state: u8) -> ExactStateId {
        ExactStateId {
            route_checkpoint_sha256: Digest([route; 32]),
            state_sha256: Digest([state; 32]),
        }
    }

    fn candidate(id: u8, predicted: Option<u64>, visits: u64) -> TacticScheduledExpansionEvidence {
        TacticScheduledExpansionEvidence {
            expansion_sha256: Digest([id; 32]),
            source_root_ticks: 100,
            source_exact_terminal_ticks_to_go: Some(20),
            generalized_terminal_support_per_million: predicted.map(|_| 900_000),
            generalized_conditional_ticks_to_go: predicted,
            uncertainty_millionths: 0,
            prediction_error_millionths: 0,
            completed_visits: visits,
            policy_rank: None,
            global_exploration_priority_rank: u64::from(id),
            source_queue_rank: u64::from(id - 1),
        }
    }

    #[test]
    fn learned_total_ticks_can_recover_the_exhaustive_best_first() {
        let decision = TacticSchedulerDecisionTrace {
            schema: crate::tactic_q_campaign::TACTIC_SCHEDULER_DECISION_SCHEMA_V1.into(),
            graph_sha256: Digest([9; 32]),
            learner_model_sha256: Digest([8; 32]),
            generation: 4,
            regime: crate::scheduler::SearchRegime::Optimization,
            queue_sha256: Digest([7; 32]),
            decision_sha256: Digest([6; 32]),
            ranked_source_queue: vec![
                candidate(1, Some(40), 0),
                candidate(2, Some(10), 5),
                candidate(3, None, 0),
            ],
            evaluated_expansion_sha256: vec![Digest([1; 32])],
            final_selected_expansion_sha256: Digest([1; 32]),
        };
        let outcomes = BTreeMap::from([
            (Digest([1; 32]), 140),
            (Digest([2; 32]), 110),
            (Digest([3; 32]), 160),
        ]);
        let report = evaluate_decision(11, 3, &decision, &outcomes).unwrap();

        assert!(report.exhaustive_surface_complete);
        assert_eq!(report.exhaustive_local_evaluations, Some(3));
        let learned = ranking(&report, NativeTacticPostTerminalControl::LearnedTotalTicks);
        assert_eq!(learned.selected_expansion_sha256, Digest([2; 32]));
        assert_eq!(learned.evaluations_to_best_observed, Some(1));
        assert_eq!(learned.selected_observed_regret_ticks, Some(0));
        assert!(decision_control_is_valid(11, &report));

        let mut tampered = report;
        tampered.rankings[0].evaluations_to_best_observed = Some(2);
        assert!(!decision_control_is_valid(11, &tampered));
    }

    #[test]
    fn incomplete_outcomes_never_claim_an_exhaustive_oracle() {
        let decision = TacticSchedulerDecisionTrace {
            schema: crate::tactic_q_campaign::TACTIC_SCHEDULER_DECISION_SCHEMA_V1.into(),
            graph_sha256: Digest([9; 32]),
            learner_model_sha256: Digest([8; 32]),
            generation: 4,
            regime: crate::scheduler::SearchRegime::Optimization,
            queue_sha256: Digest([7; 32]),
            decision_sha256: Digest([6; 32]),
            ranked_source_queue: vec![candidate(1, Some(40), 0), candidate(2, Some(10), 5)],
            evaluated_expansion_sha256: vec![Digest([1; 32])],
            final_selected_expansion_sha256: Digest([1; 32]),
        };
        let report =
            evaluate_decision(11, 3, &decision, &BTreeMap::from([(Digest([2; 32]), 110)])).unwrap();

        assert!(!report.exhaustive_surface_complete);
        assert_eq!(report.exhaustive_local_evaluations, None);
        assert!(decision_control_is_valid(11, &report));
    }

    #[test]
    fn seed_control_retains_the_exact_unleased_supported_interior_set() {
        let first = exact_state(1, 2);
        let second = exact_state(3, 4);
        let mut seed = NativeTacticPostTerminalSeedControl {
            seed: 11,
            state_graph_sha256: Digest([9; 32]),
            supported_interior_nodes: vec![first, second],
            leased_supported_interior_nodes: vec![first],
            unleased_supported_interior_nodes: vec![second],
            complete_supported_interior_coverage: false,
            optimization_decisions: 0,
            comparable_decisions: 0,
            exhaustive_complete_decisions: 0,
            learned_top_wins: 0,
            visit_top_wins: 0,
            random_top_wins: 0,
            learned_oracle_recoveries_with_fewer_evaluations: 0,
            decisions: Vec::new(),
        };
        assert!(seed_control_is_valid(&seed));

        seed.complete_supported_interior_coverage = true;
        assert!(!seed_control_is_valid(&seed));
        seed.complete_supported_interior_coverage = false;
        seed.unleased_supported_interior_nodes.clear();
        assert!(!seed_control_is_valid(&seed));
    }
}
