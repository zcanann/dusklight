//! Seeded epsilon-greedy choice over the existing live option-Q ranking.

use crate::artifact::Digest;
use crate::live_tactic_catalog::LiveTacticRanking;
use crate::option_values::OptionActionDescriptor;
use dusklight_control::option_execution::{OptionParameter, OptionType};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

mod action_factor_coverage;
mod terminal_support;

pub use action_factor_coverage::ensure_action_factor_coverage;
#[cfg(test)]
use terminal_support::action_button_mask;
pub use terminal_support::ensure_terminal_support_factor_acquisitions;

pub const TACTIC_EXPLORATION_SCHEMA_V1: &str = "dusklight-tactic-exploration/v1";
pub const EPSILON_SCALE: u32 = 1_000_000;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TacticProposalPolicy {
    Learned,
    /// Execute the learned-policy selector against the campaign's initial
    /// immutable learner snapshot while retaining experience only as shadow
    /// evidence. No update may be deployed into subsequent decisions.
    FrozenPolicy,
    RandomValid,
    StructuredNonLearning,
}

impl TacticProposalPolicy {
    pub fn uses_learned_selector(self) -> bool {
        matches!(self, Self::Learned | Self::FrozenPolicy)
    }

    pub fn deploys_policy_updates(self) -> bool {
        self == Self::Learned
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TacticExplorationConfig {
    pub seed: u64,
    pub epsilon_per_million: u32,
}

impl Default for TacticExplorationConfig {
    fn default() -> Self {
        Self {
            seed: 0,
            epsilon_per_million: 100_000,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TacticSelectionReason {
    Greedy,
    /// Follow the shortest authenticated terminal continuation already
    /// present in the exact execution graph. Learned estimates still rank
    /// unsupported alternatives, but may not displace known objective value.
    ExactTerminalReturn,
    Epsilon,
    UnsupportedBootstrap,
    BatchUncertainty,
    BatchValue,
    BatchCoverage,
    /// Acquire an unseen controller because a shared state-action outcome
    /// model predicts that its executable factors transfer productive motion
    /// from nearby authenticated outcomes.
    GeneralizedValue,
    /// Acquire an action by learned target-relative motion before the authored
    /// objective has authenticated terminal support. This is exploration
    /// evidence, not objective value or promotion authority.
    GoalReachability,
    /// Re-evaluate an untried nearby parameterization of a terminal action so
    /// route cost can improve after the first successful completion.
    TerminalCostRefinement,
    /// The graph scheduler leased this registered node/action expansion.
    GraphScheduler,
    RandomBaseline,
    StructuredBaseline,
    /// Compatibility label used by older checkpoints and callers that inject
    /// a required composition after acquisition ranking.
    BatchDiversity,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SelectedTactic {
    pub schema: String,
    pub learner_snapshot_sha256: Digest,
    pub decision_index: u64,
    pub descriptor: OptionActionDescriptor,
    pub reason: TacticSelectionReason,
    pub exploration_draw: u32,
}

pub fn choose_tactic(
    ranking: &LiveTacticRanking,
    decision_index: u64,
    config: TacticExplorationConfig,
) -> Result<SelectedTactic, TacticExplorationError> {
    choose_tactic_with_state_untried(ranking, decision_index, config, &[])
}

pub fn choose_tactic_with_state_untried(
    ranking: &LiveTacticRanking,
    decision_index: u64,
    config: TacticExplorationConfig,
    state_untried: &[OptionActionDescriptor],
) -> Result<SelectedTactic, TacticExplorationError> {
    if config.epsilon_per_million > EPSILON_SCALE
        || ranking.learner_snapshot_sha256 == Digest::ZERO
        || ranking.choices.is_empty()
    {
        return Err(TacticExplorationError::InvalidInput);
    }
    let catalog = ranking
        .choices
        .iter()
        .map(|entry| &entry.descriptor)
        .collect::<Vec<_>>();
    let available = ranking
        .choices
        .iter()
        .filter(|entry| entry.applicable)
        .map(|entry| &entry.descriptor)
        .collect::<Vec<_>>();
    let mut reported = ranking
        .values
        .ranked
        .iter()
        .map(|entry| &entry.descriptor)
        .chain(ranking.values.unsupported.iter())
        .collect::<Vec<_>>();
    reported.sort_by(|left, right| left.option_id.cmp(&right.option_id));
    if reported.len() != catalog.len()
        || catalog
            .iter()
            .any(|descriptor| reported.iter().filter(|value| *value == descriptor).count() != 1)
        || state_untried.iter().enumerate().any(|(index, descriptor)| {
            !available.contains(&descriptor) || state_untried[..index].contains(descriptor)
        })
    {
        return Err(TacticExplorationError::DetachedRanking);
    }
    if available.is_empty() {
        return Err(TacticExplorationError::NoApplicableTactic);
    }
    let ranked = ranking
        .values
        .ranked
        .iter()
        .filter(|entry| available.contains(&&entry.descriptor))
        .collect::<Vec<_>>();
    let unsupported = ranking
        .values
        .unsupported
        .iter()
        .filter(|descriptor| available.contains(descriptor))
        .cloned()
        .collect::<Vec<_>>();

    let exploration_draw =
        stratified_exploration_draw(config.seed, decision_index, config.epsilon_per_million);
    let bootstrap_unsupported = ranked.is_empty()
        || (ranked[0].mean_q <= 0.0
            && !unsupported.is_empty()
            && exploration_draw >= config.epsilon_per_million);
    let (descriptor, reason) = if exploration_draw < config.epsilon_per_million {
        // Finite tactic catalogs should spend exploratory decisions on choices
        // not yet tried in the current coarse state cell before resampling a
        // locally known action. If the caller has no state-local history, fall
        // back to globally unsupported choices and then the full live catalog.
        // Coverage is canonical and seeded, without assigning privileged
        // meaning to controller IDs, route namespaces, or action families.
        //
        // Sample the typed action class before its concrete parameterization.
        // Otherwise adding more variants of one factorized action silently
        // increases that action's exploration prior.
        let exploratory = if !state_untried.is_empty() {
            canonical_candidates(state_untried)
        } else if unsupported.is_empty() {
            let mut available = available.clone();
            available.sort_by(|left, right| left.option_id.cmp(&right.option_id));
            available
        } else {
            canonical_candidates(&unsupported)
        };
        (
            deterministic_factorized_candidate(
                &exploratory,
                config.seed,
                decision_index,
                ranking.learner_snapshot_sha256,
            )
            .clone(),
            TacticSelectionReason::Epsilon,
        )
    } else if bootstrap_unsupported {
        let unsupported = canonical_candidates(&unsupported);
        (
            deterministic_factorized_candidate(
                &unsupported,
                config.seed,
                decision_index,
                ranking.learner_snapshot_sha256,
            )
            .clone(),
            TacticSelectionReason::UnsupportedBootstrap,
        )
    } else {
        (ranked[0].descriptor.clone(), TacticSelectionReason::Greedy)
    };
    Ok(SelectedTactic {
        schema: TACTIC_EXPLORATION_SCHEMA_V1.into(),
        learner_snapshot_sha256: ranking.learner_snapshot_sha256,
        decision_index,
        descriptor,
        reason,
        exploration_draw,
    })
}

fn deterministic_factorized_candidate<'a>(
    descriptors: &[&'a OptionActionDescriptor],
    seed: u64,
    decision_index: u64,
    state: Digest,
) -> &'a OptionActionDescriptor {
    let mut groups = Vec::<(OptionType, Vec<&OptionActionDescriptor>)>::new();
    for descriptor in descriptors {
        if let Some((_, members)) = groups
            .iter_mut()
            .find(|(option_type, _)| option_type == &descriptor.option_type)
        {
            members.push(*descriptor);
        } else {
            groups.push((descriptor.option_type.clone(), vec![*descriptor]));
        }
    }
    let group_index =
        (deterministic_draw(seed, decision_index, state, 3) % groups.len() as u64) as usize;
    let members = &groups[group_index].1;
    let member_index =
        (deterministic_draw(seed, decision_index, state, 4) % members.len() as u64) as usize;
    members[member_index]
}

fn canonical_candidates(descriptors: &[OptionActionDescriptor]) -> Vec<&OptionActionDescriptor> {
    let mut candidates = descriptors.iter().collect::<Vec<_>>();
    candidates.sort_by(|left, right| left.option_id.cmp(&right.option_id));
    candidates
}

/// Choose a reproducible primary tactic plus distinct alternatives to evaluate
/// from the same learner boundary.
///
/// The first result is exactly the single-action epsilon-greedy choice. Extra
/// slots are independent acquisition lanes: ensemble uncertainty, predicted
/// value, and state-local coverage. Each lane falls back to coverage when the
/// fitted critic has no remaining supported action. This keeps unsupported
/// instances explicit while making the native batch use the critic instead of
/// merely rotating through catalog insertion order.
pub fn choose_tactic_batch_with_state_untried(
    ranking: &LiveTacticRanking,
    decision_index: u64,
    config: TacticExplorationConfig,
    state_untried: &[OptionActionDescriptor],
    maximum_proposals: usize,
) -> Result<Vec<SelectedTactic>, TacticExplorationError> {
    if maximum_proposals == 0 {
        return Err(TacticExplorationError::InvalidInput);
    }
    let primary = choose_tactic_with_state_untried(ranking, decision_index, config, state_untried)?;
    let mut result = vec![primary.clone()];
    let applicable_count = ranking
        .choices
        .iter()
        .filter(|choice| choice.applicable)
        .count();
    if maximum_proposals == 1 || applicable_count == 1 {
        return Ok(result);
    }
    // Epsilon controls which proposal leads the batch, not whether a measured
    // exploit control exists at all. Keep one greedy control whenever the
    // critic has support, then spend every remaining slot on acquisition.
    // This makes exploration outcomes directly comparable with the current
    // best action at the same native frontier.
    if let Some(greedy) = ranking.values.ranked.iter().find(|greedy| {
        ranking
            .choices
            .iter()
            .any(|choice| choice.applicable && choice.descriptor == greedy.descriptor)
    }) && greedy.descriptor != primary.descriptor
        && result.len() < maximum_proposals
    {
        result.push(SelectedTactic {
            schema: TACTIC_EXPLORATION_SCHEMA_V1.into(),
            learner_snapshot_sha256: ranking.learner_snapshot_sha256,
            decision_index,
            descriptor: greedy.descriptor.clone(),
            reason: TacticSelectionReason::BatchValue,
            exploration_draw: primary.exploration_draw,
        });
    }
    if result.len() == maximum_proposals {
        return Ok(result);
    }

    let mut candidates = ranking
        .choices
        .iter()
        .filter(|choice| choice.applicable)
        .map(|choice| choice.descriptor.clone())
        .filter(|descriptor| {
            !result
                .iter()
                .any(|proposal| proposal.descriptor == *descriptor)
        })
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return Ok(result);
    }
    candidates.sort_by(|left, right| left.option_id.cmp(&right.option_id));
    let rotation = deterministic_index(
        config.seed,
        decision_index,
        ranking.learner_snapshot_sha256,
        candidates.len(),
    );
    candidates.rotate_left(rotation);

    let mut represented_types = result
        .iter()
        .map(|proposal| proposal.descriptor.option_type.clone())
        .collect::<Vec<_>>();
    let acquisition_lanes = [
        BatchAcquisitionLane::Uncertainty,
        BatchAcquisitionLane::Value,
        BatchAcquisitionLane::Coverage,
    ];
    let mut lane_index = 0;
    while result.len() < maximum_proposals && !candidates.is_empty() {
        let lane = acquisition_lanes[lane_index % acquisition_lanes.len()];
        lane_index += 1;
        let (selected_index, selected_lane) = if let Some(index) = select_batch_candidate(
            &candidates,
            ranking,
            state_untried,
            &represented_types,
            lane,
        ) {
            (index, lane)
        } else {
            (
                select_batch_candidate(
                    &candidates,
                    ranking,
                    state_untried,
                    &represented_types,
                    BatchAcquisitionLane::Coverage,
                )
                .expect("nonempty candidate pool has a coverage selection"),
                BatchAcquisitionLane::Coverage,
            )
        };
        let descriptor = candidates.remove(selected_index);
        represented_types.push(descriptor.option_type.clone());
        result.push(SelectedTactic {
            schema: TACTIC_EXPLORATION_SCHEMA_V1.into(),
            learner_snapshot_sha256: ranking.learner_snapshot_sha256,
            decision_index,
            descriptor,
            reason: selected_lane.reason(),
            exploration_draw: (deterministic_draw(
                config.seed,
                decision_index,
                ranking.learner_snapshot_sha256,
                3 + (lane_index % 250) as u8,
            ) % u64::from(EPSILON_SCALE)) as u32,
        });
    }
    Ok(result)
}

/// Build one proposal batch for either the learned policy or an explicitly
/// non-learning baseline. All policies consume the same live applicability
/// mask and return distinct executable descriptors.
pub fn choose_tactic_batch_for_policy(
    ranking: &LiveTacticRanking,
    decision_index: u64,
    config: TacticExplorationConfig,
    state_untried: &[OptionActionDescriptor],
    maximum_proposals: usize,
    policy: TacticProposalPolicy,
) -> Result<Vec<SelectedTactic>, TacticExplorationError> {
    match policy {
        TacticProposalPolicy::Learned | TacticProposalPolicy::FrozenPolicy => {
            choose_tactic_batch_with_state_untried(
                ranking,
                decision_index,
                config,
                state_untried,
                maximum_proposals,
            )
        }
        TacticProposalPolicy::StructuredNonLearning => {
            let mut baseline = ranking.clone();
            baseline.values.ranked.clear();
            baseline.values.unsupported = baseline
                .choices
                .iter()
                .map(|choice| choice.descriptor.clone())
                .collect();
            let mut selected = choose_tactic_batch_with_state_untried(
                &baseline,
                decision_index,
                TacticExplorationConfig {
                    epsilon_per_million: 0,
                    ..config
                },
                state_untried,
                maximum_proposals,
            )?;
            for proposal in &mut selected {
                proposal.reason = TacticSelectionReason::StructuredBaseline;
            }
            Ok(selected)
        }
        TacticProposalPolicy::RandomValid => {
            choose_random_valid_batch(ranking, decision_index, config, maximum_proposals)
        }
    }
}

/// Reserve one learned-policy batch slot for local improvement after this
/// state cell has produced a terminal action.
///
/// The incumbent remains in the batch through the ordinary greedy lane. This
/// function inserts an untried, applicable descriptor in the same structural
/// family immediately after that control, displacing only the last acquisition
/// proposal when the batch is full. Separate typed parameter axes receive
/// separate acquisition lanes when both are available.
pub fn ensure_terminal_cost_refinement(
    ranking: &LiveTacticRanking,
    state_untried: &[OptionActionDescriptor],
    terminal_incumbent: Option<&OptionActionDescriptor>,
    acquisition_partition: u64,
    maximum_proposals: usize,
    proposals: &mut Vec<SelectedTactic>,
) -> Result<(), TacticExplorationError> {
    let Some(incumbent) = terminal_incumbent else {
        return Ok(());
    };
    if maximum_proposals <= 1
        || proposals.is_empty()
        || proposals.len() > maximum_proposals
        || ranking.learner_snapshot_sha256 == Digest::ZERO
    {
        return if proposals.len() <= maximum_proposals && !proposals.is_empty() {
            Ok(())
        } else {
            Err(TacticExplorationError::InvalidInput)
        };
    }
    let mut candidates = ranking
        .choices
        .iter()
        .filter(|choice| choice.applicable)
        .map(|choice| &choice.descriptor)
        .filter(|descriptor| {
            state_untried.contains(descriptor)
                && !proposals
                    .iter()
                    .any(|proposal| proposal.descriptor == **descriptor)
                && same_refinement_family(incumbent, descriptor)
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        refinement_distance(incumbent, left)
            .cmp(&refinement_distance(incumbent, right))
            .then_with(|| left.option_id.cmp(&right.option_id))
    });
    if candidates.is_empty() {
        return Ok(());
    }
    // Workers in one learned generation share the same replay boundary. Keep
    // one local improvement lane, but spread the remaining partitions across
    // the full untried parameter-distance ordering. Restricting every worker
    // to the nearest few values traps refinement in whichever cadence/phase
    // basin happened to produce the first terminal route.
    let lane = (acquisition_partition % maximum_proposals as u64) as usize;
    let incumbent_period = incumbent.parameters.get("button_pulse_period_ticks");
    let mut same_period = Vec::new();
    let mut other_period = Vec::new();
    let mut represented_periods = BTreeSet::new();
    for (index, candidate) in candidates.iter().enumerate() {
        let candidate_period = candidate.parameters.get("button_pulse_period_ticks");
        if incumbent_period.is_some() && candidate_period == incumbent_period {
            same_period.push(index);
        } else if let Some(OptionParameter::Unsigned(period)) = candidate_period {
            // Cadence acquisition owns one representative per period. The
            // distance ordering makes this the phase closest to the current
            // incumbent; phase refinement has its own lane.
            if represented_periods.insert(*period) {
                other_period.push(index);
            }
        } else {
            other_period.push(index);
        }
    }
    let split_parameter_axes = !same_period.is_empty() && !other_period.is_empty();
    let local_parameter_axes =
        single_parameter_axis_candidates(incumbent, &candidates, &same_period);
    let refinement_index = if split_parameter_axes && lane == 0 {
        // Integer controller cadences are resonant rather than smooth: one
        // period can miss a terminal while its immediate neighbor succeeds.
        // Always advance one lane through the nearest untried cadence instead
        // of allowing a fitted local response to skip it. The remaining lanes
        // still provide learned, phase/radius, and wide-interval acquisition.
        other_period[0]
    } else if split_parameter_axes && lane == 1 {
        let axis = local_parameter_axes
            .get(
                ((acquisition_partition / maximum_proposals as u64)
                    % local_parameter_axes.len().max(1) as u64) as usize,
            )
            .unwrap_or(&same_period);
        best_local_parameter_candidate(ranking, &candidates, axis).unwrap_or(axis[0])
    } else if split_parameter_axes {
        let coverage_lane = lane.saturating_sub(2);
        let coverage_lanes = maximum_proposals.saturating_sub(2);
        // The learned lane already owns the local neighborhood. Spend the
        // remaining cadence lanes at interior/far quantiles, so introducing a
        // phase axis cannot silently remove the middle of a finite interval.
        let index = coverage_lane
            .saturating_add(1)
            .saturating_mul(other_period.len() - 1)
            .div_ceil(coverage_lanes)
            .min(other_period.len() - 1);
        other_period[index]
    } else if lane == 0 {
        // The same discrete-coverage guarantee applies when cadence is the
        // only remaining axis.
        0
    } else if candidates.len() == 1 {
        0
    } else {
        lane.saturating_mul(candidates.len() - 1)
            .div_ceil(maximum_proposals - 1)
            .min(candidates.len() - 1)
    };
    let descriptor = candidates[refinement_index].clone();
    let proposal = SelectedTactic {
        schema: TACTIC_EXPLORATION_SCHEMA_V1.into(),
        learner_snapshot_sha256: ranking.learner_snapshot_sha256,
        decision_index: proposals[0].decision_index,
        descriptor,
        reason: TacticSelectionReason::TerminalCostRefinement,
        exploration_draw: proposals[0].exploration_draw,
    };
    // Keep the primary and, when epsilon led, the explicit greedy control at
    // the front. The refinement lane follows those controls and therefore
    // cannot be discarded by a later tail-replacement diversity constraint.
    let insertion_index = usize::from(
        proposals.len() > 1
            && ranking.values.ranked.first().is_some_and(|greedy| {
                proposals[1].descriptor == greedy.descriptor
                    && proposals[0].descriptor != greedy.descriptor
            }),
    ) + 1;
    proposals.insert(insertion_index.min(proposals.len()), proposal);
    proposals.truncate(maximum_proposals);
    Ok(())
}

fn best_local_parameter_candidate(
    ranking: &LiveTacticRanking,
    candidates: &[&OptionActionDescriptor],
    indices: &[usize],
) -> Option<usize> {
    indices
        .iter()
        .copied()
        .filter_map(|index| {
            local_parameter_value(ranking, candidates[index]).map(|value| (index, value))
        })
        .max_by(|left, right| {
            left.1
                .total_cmp(&right.1)
                .then_with(|| right.0.cmp(&left.0))
        })
        .map(|(index, _)| index)
}

fn single_parameter_axis_candidates(
    incumbent: &OptionActionDescriptor,
    candidates: &[&OptionActionDescriptor],
    indices: &[usize],
) -> Vec<Vec<usize>> {
    let keys = tunable_parameter_keys(incumbent);
    keys.iter()
        .copied()
        .filter(|key| *key != "button_pulse_period_ticks")
        .filter_map(|axis| {
            let members = indices
                .iter()
                .copied()
                .filter(|index| {
                    let candidate = candidates[*index];
                    candidate.parameters.get(axis) != incumbent.parameters.get(axis)
                        && keys.iter().copied().filter(|key| *key != axis).all(|key| {
                            candidate.parameters.get(key) == incumbent.parameters.get(key)
                        })
                })
                .collect::<Vec<_>>();
            (!members.is_empty()).then_some(members)
        })
        .collect()
}

/// Reserve one acquisition slot for a distinct high-level route hypothesis.
///
/// Parallel learned workers share replay and therefore need an explicit
/// partition over route families; otherwise independent seed hashes can spend
/// an entire generation on the same route while leaving another route
/// unmeasured. The existing primary/greedy control remains untouched. When
/// possible, the partitioned route inherits the primary route's cadence and
/// phase so this changes only the high-level path hypothesis.
pub fn ensure_route_family_partition(
    ranking: &LiveTacticRanking,
    state_untried: &[OptionActionDescriptor],
    acquisition_partition: u64,
    maximum_proposals: usize,
    proposals: &mut Vec<SelectedTactic>,
) -> Result<(), TacticExplorationError> {
    if maximum_proposals <= 1 {
        return Ok(());
    }
    if proposals.is_empty()
        || proposals.len() > maximum_proposals
        || ranking.learner_snapshot_sha256 == Digest::ZERO
    {
        return Err(TacticExplorationError::InvalidInput);
    }
    let families = ranking
        .choices
        .iter()
        .filter(|choice| choice.applicable)
        .filter_map(|choice| route_family_id(&choice.descriptor.option_id))
        .collect::<BTreeSet<_>>();
    if families.len() <= 1 {
        return Ok(());
    }
    let target_family = families
        .iter()
        .nth((acquisition_partition % families.len() as u64) as usize)
        .expect("nonempty route family partition");
    if proposals.iter().any(|proposal| {
        route_family_id(&proposal.descriptor.option_id).as_ref() == Some(target_family)
    }) {
        return Ok(());
    }

    let mut candidates = ranking
        .choices
        .iter()
        .filter(|choice| {
            choice.applicable
                && route_family_id(&choice.descriptor.option_id).as_ref() == Some(target_family)
                && state_untried.contains(&choice.descriptor)
        })
        .map(|choice| &choice.descriptor)
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        candidates = ranking
            .choices
            .iter()
            .filter(|choice| {
                choice.applicable
                    && route_family_id(&choice.descriptor.option_id).as_ref() == Some(target_family)
            })
            .map(|choice| &choice.descriptor)
            .collect();
    }
    candidates.sort_by(|left, right| left.option_id.cmp(&right.option_id));
    if candidates.is_empty() {
        return Ok(());
    }
    let template_suffix = proposals.iter().find_map(|proposal| {
        let family = route_family_id(&proposal.descriptor.option_id)?;
        proposal
            .descriptor
            .option_id
            .strip_prefix(&family)
            .map(str::to_owned)
    });
    let descriptor = template_suffix
        .as_deref()
        .and_then(|suffix| {
            candidates
                .iter()
                .find(|candidate| candidate.option_id.strip_prefix(target_family) == Some(suffix))
                .copied()
        })
        .unwrap_or_else(|| {
            let lower_partition = acquisition_partition / families.len() as u64;
            let index = (lower_partition.saturating_add(u64::from(proposals[0].exploration_draw))
                % candidates.len() as u64) as usize;
            candidates[index]
        })
        .clone();
    let mut acquisition = proposals[0].clone();
    acquisition.descriptor = descriptor;
    acquisition.reason = TacticSelectionReason::BatchCoverage;
    proposals.insert(1, acquisition);
    proposals.truncate(maximum_proposals);
    Ok(())
}

/// Reserve one learned acquisition slot for a bounded composition of the
/// current terminal route with another compatible route hypothesis.
///
/// Crossovers are materialized state-locally by the route catalog. This
/// selector makes the composition executable promptly instead of relying on a
/// large unsupported catalog lottery.
pub fn ensure_route_composition_refinement(
    ranking: &LiveTacticRanking,
    state_untried: &[OptionActionDescriptor],
    terminal_incumbent: Option<&OptionActionDescriptor>,
    acquisition_partition: u64,
    maximum_proposals: usize,
    proposals: &mut Vec<SelectedTactic>,
) -> Result<(), TacticExplorationError> {
    let Some(incumbent) = terminal_incumbent else {
        return Ok(());
    };
    let Some(incumbent_family) = route_family_id(&incumbent.option_id) else {
        return Ok(());
    };
    if maximum_proposals <= 1 || proposals.is_empty() || proposals.len() > maximum_proposals {
        return if !proposals.is_empty() && proposals.len() <= maximum_proposals {
            Ok(())
        } else {
            Err(TacticExplorationError::InvalidInput)
        };
    }
    let incumbent_route = incumbent_family
        .strip_prefix("goal.seek.route.")
        .expect("route family prefix");
    let crossover_suffix = format!(".crossover.{incumbent_route}.");
    let mut candidates = ranking
        .choices
        .iter()
        .filter(|choice| choice.applicable)
        .map(|choice| &choice.descriptor)
        .filter(|descriptor| {
            descriptor.option_id.contains(".crossover.")
                && (descriptor
                    .option_id
                    .starts_with(&format!("{incumbent_family}.crossover."))
                    || descriptor.option_id.contains(&crossover_suffix))
                && state_untried.contains(descriptor)
                && !proposals
                    .iter()
                    .any(|proposal| proposal.descriptor == **descriptor)
        })
        .collect::<Vec<_>>();
    // Crossovers couple discrete geometry, cadence, and lookahead axes. A base
    // route spreads coverage across the faster cadences first. Once a
    // crossover itself wins, keep its exact geometry and cadence fixed while
    // trying lookahead variants before returning to other compositions.
    if incumbent.option_id.contains(".crossover.") {
        let incumbent_structure = incumbent
            .option_id
            .split_once(".roll.period.")
            .map(|(structure, _)| structure)
            .unwrap_or(&incumbent.option_id);
        let incumbent_period = incumbent.parameters.get("button_pulse_period_ticks");
        candidates.sort_by(|left, right| {
            let left_structure = left
                .option_id
                .split_once(".roll.period.")
                .map(|(structure, _)| structure)
                .unwrap_or(&left.option_id);
            let right_structure = right
                .option_id
                .split_once(".roll.period.")
                .map(|(structure, _)| structure)
                .unwrap_or(&right.option_id);
            let left_exact = left_structure == incumbent_structure;
            let right_exact = right_structure == incumbent_structure;
            let left_same_period =
                left.parameters.get("button_pulse_period_ticks") == incumbent_period;
            let right_same_period =
                right.parameters.get("button_pulse_period_ticks") == incumbent_period;
            right_same_period
                .cmp(&left_same_period)
                // Interpolated siblings are the new structural information;
                // the exact hard crossover already has a measured incumbent.
                .then_with(|| left_exact.cmp(&right_exact))
                .then_with(|| left.option_id.cmp(&right.option_id))
        });
    } else {
        candidates.sort_by(|left, right| {
            let period = |descriptor: &OptionActionDescriptor| {
                descriptor
                    .parameters
                    .get("button_pulse_period_ticks")
                    .and_then(|parameter| match parameter {
                        OptionParameter::Unsigned(period) => Some(*period),
                        _ => None,
                    })
                    .unwrap_or(u64::MAX)
            };
            // The incumbent already proves its terminal suffix. Prefer
            // replacing the approach prefix while preserving that suffix
            // before replacing the successful exit with an unsupported peer.
            // Within that structural priority, try faster cadence and stable
            // option order.
            let left_preserves_terminal_suffix = left.option_id.contains(&crossover_suffix);
            let right_preserves_terminal_suffix = right.option_id.contains(&crossover_suffix);
            right_preserves_terminal_suffix
                .cmp(&left_preserves_terminal_suffix)
                .then_with(|| period(left).cmp(&period(right)))
                .then_with(|| left.option_id.cmp(&right.option_id))
        });
    }
    if candidates.is_empty() {
        return Ok(());
    }
    let descriptor = candidates[(acquisition_partition % candidates.len() as u64) as usize].clone();
    let mut composition = proposals[0].clone();
    composition.descriptor = descriptor;
    composition.reason = TacticSelectionReason::BatchDiversity;
    let insertion_index = usize::from(
        proposals.len() > 1
            && ranking.values.ranked.first().is_some_and(|greedy| {
                proposals[1].descriptor == greedy.descriptor
                    && proposals[0].descriptor != greedy.descriptor
            }),
    ) + 1;
    proposals.insert(insertion_index.min(proposals.len()), composition);
    proposals.truncate(maximum_proposals);
    Ok(())
}

const MAX_GENERALIZED_VALUE_ACQUISITION_RANKS: usize = 128;

/// Reserve one policy slot for a partitioned, high-ranked applicable
/// controller from a shared state-action outcome model.
///
/// The caller supplies descriptors in predicted-outcome order. This selector
/// deliberately knows nothing about controller IDs, route families, or
/// hand-authored tactic semantics. Parallel workers partition the top-ranked
/// window instead of all evaluating rank zero from the same shared model. A
/// supported action remains eligible: learning would not be an exploit-capable
/// policy if evidence that an action worked made that action ineligible.
pub fn ensure_generalized_value_acquisition(
    ranked_applicable: &[OptionActionDescriptor],
    acquisition_partition: u64,
    maximum_proposals: usize,
    proposals: &mut Vec<SelectedTactic>,
) -> Result<(), TacticExplorationError> {
    ensure_ranked_model_acquisition(
        ranked_applicable,
        acquisition_partition,
        maximum_proposals,
        TacticSelectionReason::GeneralizedValue,
        proposals,
    )
}

/// Reserve the authoritative pre-terminal policy slot for the critic's best
/// learned reachability prediction.
///
/// Native siblings and graph leases already provide diverse exploration
/// evidence. A worker/lane partition must not silently turn rank N into the
/// retained policy action at a different state.
pub fn ensure_goal_reachability_acquisition(
    ranked_applicable: &[OptionActionDescriptor],
    _acquisition_partition: u64,
    maximum_proposals: usize,
    proposals: &mut Vec<SelectedTactic>,
) -> Result<(), TacticExplorationError> {
    if maximum_proposals == 0 || proposals.is_empty() || proposals.len() > maximum_proposals {
        return if !proposals.is_empty() && proposals.len() <= maximum_proposals {
            Ok(())
        } else {
            Err(TacticExplorationError::InvalidInput)
        };
    }
    let Some(descriptor) = ranked_applicable.first() else {
        return Ok(());
    };
    ensure_model_acquisition(
        descriptor,
        maximum_proposals,
        TacticSelectionReason::GoalReachability,
        proposals,
    );
    Ok(())
}

fn ensure_ranked_model_acquisition(
    ranked_applicable: &[OptionActionDescriptor],
    acquisition_partition: u64,
    maximum_proposals: usize,
    reason: TacticSelectionReason,
    proposals: &mut Vec<SelectedTactic>,
) -> Result<(), TacticExplorationError> {
    if maximum_proposals == 0 || proposals.is_empty() || proposals.len() > maximum_proposals {
        return if !proposals.is_empty() && proposals.len() <= maximum_proposals {
            Ok(())
        } else {
            Err(TacticExplorationError::InvalidInput)
        };
    }
    let candidates = interleave_ranked_action_types(ranked_applicable)
        .into_iter()
        .take(MAX_GENERALIZED_VALUE_ACQUISITION_RANKS)
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return Ok(());
    }
    let descriptor = candidates[(acquisition_partition % candidates.len() as u64) as usize];
    ensure_model_acquisition(descriptor, maximum_proposals, reason, proposals);
    Ok(())
}

fn ensure_model_acquisition(
    descriptor: &OptionActionDescriptor,
    maximum_proposals: usize,
    reason: TacticSelectionReason,
    proposals: &mut Vec<SelectedTactic>,
) {
    if let Some(existing) = proposals
        .iter_mut()
        .find(|proposal| proposal.descriptor == *descriptor)
    {
        if existing.reason != TacticSelectionReason::Epsilon {
            existing.reason = reason;
        }
        return;
    }
    let mut acquisition = proposals[0].clone();
    acquisition.descriptor = descriptor.clone();
    acquisition.reason = reason;
    if maximum_proposals == 1 {
        // Proposal width limits parallel sibling evaluation, not whether a
        // learned policy can act. Preserve explicit epsilon exploration and
        // an exact supported greedy choice, but let a state-action model
        // replace an unsupported bootstrap even on a single-worker lane.
        if !matches!(
            proposals[0].reason,
            TacticSelectionReason::Epsilon | TacticSelectionReason::Greedy
        ) {
            proposals[0] = acquisition;
        }
        return;
    }
    proposals.insert(1, acquisition);
    proposals.truncate(maximum_proposals);
}

fn interleave_ranked_action_types(
    ranked: &[OptionActionDescriptor],
) -> Vec<&OptionActionDescriptor> {
    let mut groups = Vec::<(OptionType, Vec<&OptionActionDescriptor>)>::new();
    for descriptor in ranked {
        if let Some((_, members)) = groups
            .iter_mut()
            .find(|(option_type, _)| option_type == &descriptor.option_type)
        {
            members.push(descriptor);
        } else {
            groups.push((descriptor.option_type.clone(), vec![descriptor]));
        }
    }
    interleave_ranked_groups(&groups)
}

fn interleave_ranked_groups<'a, T>(
    groups: &[(T, Vec<&'a OptionActionDescriptor>)],
) -> Vec<&'a OptionActionDescriptor> {
    let maximum_group = groups
        .iter()
        .map(|(_, members)| members.len())
        .max()
        .unwrap_or(0);
    let mut interleaved = Vec::with_capacity(groups.iter().map(|(_, members)| members.len()).sum());
    for rank_within_type in 0..maximum_group {
        for (_, members) in groups {
            if let Some(descriptor) = members.get(rank_within_type) {
                interleaved.push(*descriptor);
            }
        }
    }
    interleaved
}

/// Make the shared model's partitioned acquisition authoritative when the
/// exact policy lacks support, while preserving an exploratory or supported
/// exact primary.
pub fn retain_generalized_value_acquisition(
    proposals: &mut [SelectedTactic],
) -> Result<(), TacticExplorationError> {
    retain_ranked_model_acquisition(proposals, TacticSelectionReason::GeneralizedValue)
}

pub fn retain_goal_reachability_acquisition(
    proposals: &mut [SelectedTactic],
) -> Result<(), TacticExplorationError> {
    retain_ranked_model_acquisition(proposals, TacticSelectionReason::GoalReachability)
}

fn retain_ranked_model_acquisition(
    proposals: &mut [SelectedTactic],
    reason: TacticSelectionReason,
) -> Result<(), TacticExplorationError> {
    if proposals.is_empty() {
        return Err(TacticExplorationError::InvalidInput);
    }
    // Epsilon controls behavior, not merely which siblings are measured, and
    // a supported exact greedy action is stronger evidence than an
    // interpolated state-action estimate. Keep the generalized candidate in
    // the same native batch as a control without silently replacing either
    // authoritative choice.
    if matches!(
        proposals[0].reason,
        TacticSelectionReason::Epsilon | TacticSelectionReason::Greedy
    ) {
        return Ok(());
    }
    let Some(index) = proposals
        .iter()
        .position(|proposal| proposal.reason == reason)
    else {
        return Ok(());
    };
    proposals.swap(0, index);
    Ok(())
}

fn choose_random_valid_batch(
    ranking: &LiveTacticRanking,
    decision_index: u64,
    config: TacticExplorationConfig,
    maximum_proposals: usize,
) -> Result<Vec<SelectedTactic>, TacticExplorationError> {
    if maximum_proposals == 0
        || config.epsilon_per_million > EPSILON_SCALE
        || ranking.learner_snapshot_sha256 == Digest::ZERO
        || ranking.choices.is_empty()
    {
        return Err(TacticExplorationError::InvalidInput);
    }
    let mut descriptors = ranking
        .choices
        .iter()
        .filter(|choice| choice.applicable)
        .map(|choice| choice.descriptor.clone())
        .collect::<Vec<_>>();
    if descriptors.is_empty() {
        return Err(TacticExplorationError::NoApplicableTactic);
    }
    descriptors.sort_by(|left, right| {
        random_baseline_key(
            config.seed,
            decision_index,
            ranking.learner_snapshot_sha256,
            &left.option_id,
        )
        .cmp(&random_baseline_key(
            config.seed,
            decision_index,
            ranking.learner_snapshot_sha256,
            &right.option_id,
        ))
        .then_with(|| left.option_id.cmp(&right.option_id))
    });
    Ok(descriptors
        .into_iter()
        .take(maximum_proposals)
        .enumerate()
        .map(|(index, descriptor)| SelectedTactic {
            schema: TACTIC_EXPLORATION_SCHEMA_V1.into(),
            learner_snapshot_sha256: ranking.learner_snapshot_sha256,
            decision_index,
            descriptor,
            reason: TacticSelectionReason::RandomBaseline,
            exploration_draw: (random_baseline_key(
                config.seed,
                decision_index,
                ranking.learner_snapshot_sha256,
                &format!("draw/{index}"),
            ) % u64::from(EPSILON_SCALE)) as u32,
        })
        .collect())
}

fn random_baseline_key(seed: u64, decision_index: u64, state: Digest, option_id: &str) -> u64 {
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight-random-valid-tactic-baseline/v1");
    hasher.update(seed.to_le_bytes());
    hasher.update(decision_index.to_le_bytes());
    hasher.update(state.0);
    hasher.update((option_id.len() as u64).to_le_bytes());
    hasher.update(option_id.as_bytes());
    let digest = hasher.finalize();
    u64::from_le_bytes(digest[..8].try_into().unwrap())
}

#[derive(Clone, Copy)]
enum BatchAcquisitionLane {
    Uncertainty,
    Value,
    Coverage,
}

impl BatchAcquisitionLane {
    fn reason(self) -> TacticSelectionReason {
        match self {
            Self::Uncertainty => TacticSelectionReason::BatchUncertainty,
            Self::Value => TacticSelectionReason::BatchValue,
            Self::Coverage => TacticSelectionReason::BatchCoverage,
        }
    }
}

fn select_batch_candidate(
    candidates: &[OptionActionDescriptor],
    ranking: &LiveTacticRanking,
    state_untried: &[OptionActionDescriptor],
    represented_types: &[OptionType],
    lane: BatchAcquisitionLane,
) -> Option<usize> {
    candidates
        .iter()
        .enumerate()
        .filter_map(|(index, descriptor)| {
            let untried = state_untried.contains(descriptor);
            let estimate = ranking
                .values
                .ranked
                .iter()
                .find(|ranked| ranked.descriptor == *descriptor);
            // The primary slot already exploits the fitted critic. Additional
            // acquisition lanes must not spend native ticks remeasuring an
            // action already observed in this state cell. Uncertainty and
            // value remain useful for actions learned in other states; when
            // no such state-local untried action exists, the caller falls back
            // to explicit coverage of a new executable descriptor.
            if !matches!(lane, BatchAcquisitionLane::Coverage) && (estimate.is_none() || !untried) {
                return None;
            }
            let unsupported = ranking.values.unsupported.contains(descriptor);
            let novel_type = !represented_types.contains(&descriptor.option_type);
            let mean_q = estimate.map_or(f64::NEG_INFINITY, |ranked| ranked.mean_q);
            let uncertainty = estimate.map_or(f64::NEG_INFINITY, |ranked| ranked.ensemble_variance);
            Some((
                index,
                match lane {
                    BatchAcquisitionLane::Uncertainty => {
                        (uncertainty, mean_q, untried, novel_type, unsupported)
                    }
                    BatchAcquisitionLane::Value => {
                        (mean_q, uncertainty, untried, novel_type, unsupported)
                    }
                    BatchAcquisitionLane::Coverage => (
                        f64::from(u8::from(untried)),
                        f64::from(u8::from(unsupported)),
                        novel_type,
                        unsupported,
                        untried,
                    ),
                },
            ))
        })
        .max_by(|left, right| {
            left.1
                .0
                .total_cmp(&right.1.0)
                .then_with(|| left.1.1.total_cmp(&right.1.1))
                .then_with(|| left.1.2.cmp(&right.1.2))
                .then_with(|| left.1.3.cmp(&right.1.3))
                .then_with(|| left.1.4.cmp(&right.1.4))
                // `candidates` was already deterministically rotated for this
                // seed and boundary. Preserve that seeded coverage order
                // instead of undoing it with an option-ID tie-break.
                .then_with(|| right.0.cmp(&left.0))
        })
        .map(|(index, _)| index)
}

fn is_route_sequence(descriptor: &OptionActionDescriptor) -> bool {
    descriptor.parameters.contains_key("coordinates")
        // Layered route compositions use the generic reactive-controller
        // adapter, whose descriptor intentionally exposes only the canonical
        // program identity and duration. The goal-conditioned catalog keeps
        // every route-derived action under this stable namespace, including
        // button overlays such as rolling, so they must remain in the same
        // exploration class as their native coordinate-sequence counterparts.
        || descriptor.option_id.starts_with("goal.seek.route.")
}

fn route_family_id(option_id: &str) -> Option<String> {
    let suffix = option_id.strip_prefix("goal.seek.route.")?;
    let route = suffix.split('.').next()?;
    (!route.is_empty()).then(|| format!("goal.seek.route.{route}"))
}

fn same_refinement_family(
    incumbent: &OptionActionDescriptor,
    candidate: &OptionActionDescriptor,
) -> bool {
    if is_route_sequence(incumbent) || is_route_sequence(candidate) {
        if !is_route_sequence(incumbent) || !is_route_sequence(candidate) {
            return false;
        }
        return match (
            controller_refinement_identity(incumbent),
            controller_refinement_identity(candidate),
        ) {
            (Some(left), Some(right)) => left == right,
            (None, None) => incumbent.option_type == candidate.option_type,
            _ => false,
        };
    }
    incumbent.option_type == candidate.option_type
        && tunable_parameter_keys(incumbent) == tunable_parameter_keys(candidate)
}

fn controller_refinement_identity(descriptor: &OptionActionDescriptor) -> Option<&OptionParameter> {
    descriptor
        .parameters
        .get("controller_structure_sha256")
        .or_else(|| descriptor.parameters.get("controller_base_sha256"))
}

fn tunable_parameter_keys(descriptor: &OptionActionDescriptor) -> Vec<&str> {
    descriptor
        .parameters
        .keys()
        .map(String::as_str)
        .filter(|key| {
            !matches!(
                *key,
                "program_sha256"
                    | "controller_base_sha256"
                    | "controller_structure_sha256"
                    | "duration_ticks"
            )
        })
        .collect()
}

fn refinement_distance(
    incumbent: &OptionActionDescriptor,
    candidate: &OptionActionDescriptor,
) -> u128 {
    let mut distance = 0_u128;
    let keys = tunable_parameter_keys(incumbent);
    for key in keys {
        let Some(left) = incumbent.parameters.get(key) else {
            continue;
        };
        let Some(right) = candidate.parameters.get(key) else {
            return u128::MAX;
        };
        distance = distance.saturating_add(parameter_distance(left, right));
    }
    distance
}

fn parameter_distance(left: &OptionParameter, right: &OptionParameter) -> u128 {
    match (left, right) {
        (OptionParameter::Unsigned(left), OptionParameter::Unsigned(right)) => {
            u128::from(left.abs_diff(*right))
        }
        (OptionParameter::Signed(left), OptionParameter::Signed(right)) => {
            u128::from(left.abs_diff(*right))
        }
        (OptionParameter::F32Bits(left), OptionParameter::F32Bits(right)) => {
            let left = f32::from_bits(*left);
            let right = f32::from_bits(*right);
            if left.is_finite() && right.is_finite() {
                ((left - right).abs() * 1_000_000.0).round() as u128
            } else {
                u128::MAX
            }
        }
        (left, right) => u128::from(left != right),
    }
}

/// Predict an unseen parameterization from fitted values in the same exact
/// structural family. This is deliberately acquisition-only: descriptors
/// from another controller base or with another typed parameter schema remain
/// unsupported and never receive an invented critic value.
fn local_parameter_value(
    ranking: &LiveTacticRanking,
    candidate: &OptionActionDescriptor,
) -> Option<f64> {
    let keys = tunable_parameter_keys(candidate);
    let family = ranking
        .values
        .ranked
        .iter()
        .filter(|value| {
            same_refinement_family(candidate, &value.descriptor)
                && tunable_parameter_keys(&value.descriptor) == keys
        })
        .collect::<Vec<_>>();
    if family.len() < 2 {
        return None;
    }
    let family_choices = ranking
        .choices
        .iter()
        .filter(|choice| {
            same_refinement_family(candidate, &choice.descriptor)
                && tunable_parameter_keys(&choice.descriptor) == keys
        })
        .map(|choice| &choice.descriptor)
        .collect::<Vec<_>>();
    let scales = keys
        .iter()
        .filter_map(|key| {
            let values = family_choices
                .iter()
                .filter_map(|descriptor| descriptor.parameters.get(*key))
                .filter_map(numeric_parameter)
                .collect::<Vec<_>>();
            let minimum = values.iter().copied().min_by(f64::total_cmp)?;
            let maximum = values.iter().copied().max_by(f64::total_cmp)?;
            let range = maximum - minimum;
            (range > f64::EPSILON && range.is_finite()).then_some((*key, range))
        })
        .collect::<Vec<_>>();
    if scales.is_empty() {
        return None;
    }
    let mut neighbors = family
        .into_iter()
        .filter_map(|value| {
            let distance = normalized_parameter_distance(candidate, &value.descriptor, &scales)?;
            (distance > 0.0 && distance.is_finite() && value.mean_q.is_finite())
                .then_some((distance, value.mean_q))
        })
        .collect::<Vec<_>>();
    if neighbors.len() < 2 {
        return None;
    }
    neighbors.sort_by(|left, right| left.0.total_cmp(&right.0));
    neighbors.truncate(4);
    let (weighted_value, total_weight) =
        neighbors
            .into_iter()
            .fold((0.0_f64, 0.0_f64), |(value, weight), (distance, q)| {
                let neighbor_weight = 1.0 / distance.max(1.0e-6);
                (value + neighbor_weight * q, weight + neighbor_weight)
            });
    (total_weight > 0.0)
        .then_some(weighted_value / total_weight)
        .filter(|value| value.is_finite())
}

fn normalized_parameter_distance(
    left: &OptionActionDescriptor,
    right: &OptionActionDescriptor,
    scales: &[(&str, f64)],
) -> Option<f64> {
    scales.iter().try_fold(0.0_f64, |distance, (key, scale)| {
        let left = numeric_parameter(left.parameters.get(*key)?)?;
        let right = numeric_parameter(right.parameters.get(*key)?)?;
        let delta = (left - right) / scale;
        Some(delta.mul_add(delta, distance))
    })
}

fn numeric_parameter(parameter: &OptionParameter) -> Option<f64> {
    match parameter {
        OptionParameter::Bool(value) => Some(f64::from(u8::from(*value))),
        OptionParameter::Signed(value) => Some(*value as f64),
        OptionParameter::Unsigned(value) => Some(*value as f64),
        OptionParameter::F32Bits(bits) => {
            let value = f32::from_bits(*bits);
            value.is_finite().then_some(f64::from(value))
        }
        OptionParameter::Vec3F32Bits(_) | OptionParameter::Text(_) | OptionParameter::Digest(_) => {
            None
        }
    }
}

fn deterministic_index(seed: u64, decision_index: u64, state: Digest, len: usize) -> usize {
    (deterministic_draw(seed, decision_index, state, 1) % len as u64) as usize
}

/// Schedules epsilon decisions at their declared density with a seeded phase.
///
/// Independent Bernoulli draws can legally produce an arbitrarily long greedy
/// streak, which makes short native campaigns depend on luck rather than their
/// configured exploration rate. Accumulating epsilon through a fixed-size
/// cycle retains deterministic epsilon-greedy selection while bounding the gap
/// between exploration decisions whenever epsilon divides the scale.
fn stratified_exploration_draw(seed: u64, decision_index: u64, epsilon: u32) -> u32 {
    let phase = deterministic_draw(seed, 0, Digest::ZERO, 2) % u64::from(EPSILON_SCALE);
    let offset = u128::from(decision_index) * u128::from(epsilon);
    ((u128::from(phase) + offset) % u128::from(EPSILON_SCALE)) as u32
}

fn deterministic_draw(seed: u64, decision_index: u64, state: Digest, lane: u8) -> u64 {
    let mut hasher = Sha256::new();
    hasher.update(TACTIC_EXPLORATION_SCHEMA_V1.as_bytes());
    hasher.update(seed.to_le_bytes());
    hasher.update(decision_index.to_le_bytes());
    hasher.update(state.0);
    hasher.update([lane]);
    let digest = hasher.finalize();
    u64::from_le_bytes(digest[..8].try_into().unwrap())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TacticExplorationError {
    InvalidInput,
    DetachedRanking,
    NoApplicableTactic,
}

impl fmt::Display for TacticExplorationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput => formatter.write_str("tactic exploration input is invalid"),
            Self::DetachedRanking => {
                formatter.write_str("tactic ranking is detached from its live catalog")
            }
            Self::NoApplicableTactic => {
                formatter.write_str("tactic ranking has no applicable action")
            }
        }
    }
}

impl Error for TacticExplorationError {}

#[cfg(test)]
mod tests;
