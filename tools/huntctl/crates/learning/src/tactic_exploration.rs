//! Seeded epsilon-greedy choice over the existing live option-Q ranking.

use crate::artifact::Digest;
use crate::live_tactic_catalog::LiveTacticRanking;
use crate::option_values::OptionActionDescriptor;
use dusklight_control::option_execution::{OptionParameter, OptionType};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;

pub const TACTIC_EXPLORATION_SCHEMA_V1: &str = "dusklight-tactic-exploration/v1";
pub const EPSILON_SCALE: u32 = 1_000_000;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TacticProposalPolicy {
    Learned,
    RandomValid,
    StructuredNonLearning,
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
    Epsilon,
    UnsupportedBootstrap,
    BatchUncertainty,
    BatchValue,
    BatchCoverage,
    /// Re-evaluate an untried nearby parameterization of a terminal action so
    /// route cost can improve after the first successful completion.
    TerminalCostRefinement,
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
    let available = ranking
        .choices
        .iter()
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
    if reported.len() != available.len()
        || available
            .iter()
            .any(|descriptor| reported.iter().filter(|value| *value == descriptor).count() != 1)
        || state_untried.iter().enumerate().any(|(index, descriptor)| {
            !available.contains(&descriptor) || state_untried[..index].contains(descriptor)
        })
    {
        return Err(TacticExplorationError::DetachedRanking);
    }

    let exploration_draw =
        stratified_exploration_draw(config.seed, decision_index, config.epsilon_per_million);
    let supported_route = ranking
        .values
        .ranked
        .iter()
        .any(|value| is_route_sequence(&value.descriptor));
    let bootstrap_unsupported = ranking.values.ranked.is_empty()
        || (ranking.values.ranked[0].mean_q <= 0.0
            && !ranking.values.unsupported.is_empty()
            && exploration_draw >= config.epsilon_per_million);
    let (descriptor, reason) = if bootstrap_unsupported {
        let unsupported = prioritized_unsupported(&ranking.values.unsupported, supported_route);
        let index = deterministic_index(
            config.seed,
            decision_index,
            ranking.learner_snapshot_sha256,
            unsupported.len(),
        );
        (
            unsupported[index].clone(),
            TacticSelectionReason::UnsupportedBootstrap,
        )
    } else if exploration_draw < config.epsilon_per_million {
        // Finite tactic catalogs should spend exploratory decisions on choices
        // not yet tried in the current coarse state cell before resampling a
        // locally known action. If the caller has no state-local history, fall
        // back to globally unsupported choices and then the full live catalog.
        // Prefer typed spatial targets, long full-strength heading probes, and
        // bounded curves. The first exploit the goal-relative corridor; the
        // others make lateral detours around contact geometry discoverable
        // without prioritizing every short control variant. This is still
        // epsilon-greedy—the greedy branch is unchanged.
        let exploratory = if !state_untried.is_empty() {
            prioritized_unsupported(state_untried, supported_route)
        } else if ranking.values.unsupported.is_empty() {
            available
        } else {
            prioritized_unsupported(&ranking.values.unsupported, supported_route)
        };
        let index = deterministic_index(
            config.seed,
            decision_index,
            ranking.learner_snapshot_sha256,
            exploratory.len(),
        );
        (exploratory[index].clone(), TacticSelectionReason::Epsilon)
    } else {
        (
            ranking.values.ranked[0].descriptor.clone(),
            TacticSelectionReason::Greedy,
        )
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
    if maximum_proposals == 1 || ranking.choices.len() == 1 {
        return Ok(result);
    }
    // Epsilon controls which proposal leads the batch, not whether a measured
    // exploit control exists at all. Keep one greedy control whenever the
    // critic has support, then spend every remaining slot on acquisition.
    // This makes exploration outcomes directly comparable with the current
    // best action at the same native frontier.
    if let Some(greedy) = ranking.values.ranked.first()
        && greedy.descriptor != primary.descriptor
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
        TacticProposalPolicy::Learned => choose_tactic_batch_with_state_untried(
            ranking,
            decision_index,
            config,
            state_untried,
            maximum_proposals,
        ),
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
/// function inserts the closest untried, applicable descriptor in the same
/// structural family immediately after that control, displacing only the last
/// acquisition proposal when the batch is full.
pub fn ensure_terminal_cost_refinement(
    ranking: &LiveTacticRanking,
    state_untried: &[OptionActionDescriptor],
    terminal_incumbent: Option<&OptionActionDescriptor>,
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
    let Some(descriptor) = candidates.first().copied().cloned() else {
        return Ok(());
    };
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
                descriptor.option_id.as_str(),
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
                .then_with(|| right.2.cmp(left.2))
        })
        .map(|(index, _, _)| index)
}

fn prioritized_unsupported(
    unsupported: &[OptionActionDescriptor],
    escape_before_routes: bool,
) -> Vec<&OptionActionDescriptor> {
    let route_sequences = unsupported
        .iter()
        .filter(|descriptor| is_route_sequence(descriptor))
        .collect::<Vec<_>>();
    let escape_actions = unsupported
        .iter()
        .filter(|descriptor| {
            matches!(
                descriptor.option_type,
                OptionType::Roll
                    | OptionType::Interact
                    | OptionType::Attack
                    | OptionType::JumpAttack
            )
        })
        .collect::<Vec<_>>();
    if escape_before_routes && !escape_actions.is_empty() {
        let semantic_escape_actions = escape_actions
            .iter()
            .copied()
            .filter(|descriptor| descriptor.option_type != OptionType::Roll)
            .collect::<Vec<_>>();
        if !semantic_escape_actions.is_empty() {
            return semantic_escape_actions;
        }
        return escape_actions;
    }
    if !route_sequences.is_empty() {
        return route_sequences;
    }
    if !escape_actions.is_empty() {
        return escape_actions;
    }
    let navigation = unsupported
        .iter()
        .filter(|descriptor| {
            descriptor.parameters.contains_key("coordinate")
                || descriptor.parameters.contains_key("control")
                || (descriptor.parameters.contains_key("heading_radians")
                    && matches!(
                        descriptor.parameters.get("magnitude"),
                        Some(OptionParameter::Unsigned(127))
                    )
                    && matches!(
                        descriptor.parameters.get("maximum_ticks"),
                        Some(OptionParameter::Unsigned(16))
                    ))
        })
        .collect::<Vec<_>>();
    if navigation.is_empty() {
        unsupported.iter().collect()
    } else {
        navigation
    }
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

fn same_refinement_family(
    incumbent: &OptionActionDescriptor,
    candidate: &OptionActionDescriptor,
) -> bool {
    if is_route_sequence(incumbent) || is_route_sequence(candidate) {
        if !is_route_sequence(incumbent) || !is_route_sequence(candidate) {
            return false;
        }
        return match (
            incumbent.parameters.get("controller_base_sha256"),
            candidate.parameters.get("controller_base_sha256"),
        ) {
            (Some(left), Some(right)) => left == right,
            (None, None) => incumbent.option_type == candidate.option_type,
            _ => false,
        };
    }
    incumbent.option_type == candidate.option_type
        && tunable_parameter_keys(incumbent) == tunable_parameter_keys(candidate)
}

fn tunable_parameter_keys(descriptor: &OptionActionDescriptor) -> Vec<&str> {
    descriptor
        .parameters
        .keys()
        .map(String::as_str)
        .filter(|key| {
            !matches!(
                *key,
                "program_sha256" | "controller_base_sha256" | "duration_ticks"
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
mod tests {
    use super::*;
    use crate::learner_state::LearnerActionMaskEntry;
    use crate::option_values::{AvailableOptionRanking, RankedOption};
    use crate::tactic_asset::TacticDurationBounds;
    use crate::tactic_blueprint::ConcreteTacticChoiceKind;
    use dusklight_control::option_execution::{OptionParameter, OptionType};
    use std::collections::BTreeMap;

    fn descriptor(id: &str, option_type: OptionType) -> OptionActionDescriptor {
        OptionActionDescriptor {
            option_id: id.into(),
            option_type,
            parameters: BTreeMap::new(),
        }
    }

    fn choice(descriptor: OptionActionDescriptor) -> LearnerActionMaskEntry {
        LearnerActionMaskEntry {
            choice_id: descriptor.option_id.clone(),
            kind: ConcreteTacticChoiceKind::CatalogEntry,
            descriptor,
            duration: TacticDurationBounds {
                minimum_ticks: 1,
                maximum_ticks: 1,
            },
            applicable: true,
        }
    }

    #[test]
    fn zero_epsilon_is_greedy_and_seeded_exploration_is_reproducible() {
        let wait = descriptor("wait", OptionType::Neutral);
        let roll = descriptor("roll", OptionType::Roll);
        let ranking = LiveTacticRanking {
            learner_snapshot_sha256: Digest([1; 32]),
            action_universe_sha256: Digest([2; 32]),
            choices: vec![choice(wait.clone()), choice(roll.clone())],
            values: AvailableOptionRanking {
                ranked: vec![
                    RankedOption {
                        action_id: 1,
                        descriptor: roll.clone(),
                        mean_q: 5.0,
                        ensemble_variance: 0.0,
                    },
                    RankedOption {
                        action_id: 0,
                        descriptor: wait,
                        mean_q: 1.0,
                        ensemble_variance: 0.0,
                    },
                ],
                unsupported: Vec::new(),
            },
        };
        let greedy = choose_tactic(
            &ranking,
            7,
            TacticExplorationConfig {
                seed: 99,
                epsilon_per_million: 0,
            },
        )
        .unwrap();
        assert_eq!(greedy.descriptor, roll);
        assert_eq!(greedy.reason, TacticSelectionReason::Greedy);

        let config = TacticExplorationConfig {
            seed: 99,
            epsilon_per_million: EPSILON_SCALE,
        };
        assert_eq!(
            choose_tactic(&ranking, 7, config).unwrap(),
            choose_tactic(&ranking, 7, config).unwrap()
        );
        assert_eq!(
            choose_tactic(&ranking, 7, config).unwrap().reason,
            TacticSelectionReason::Epsilon
        );
    }

    #[test]
    fn stratified_epsilon_bounds_finite_campaign_exploration_gaps() {
        for seed in 0..32 {
            let quarter = (0..20)
                .map(|decision| stratified_exploration_draw(seed, decision, 250_000))
                .collect::<Vec<_>>();
            for cycle in quarter.chunks_exact(4) {
                assert_eq!(cycle.iter().filter(|draw| **draw < 250_000).count(), 1);
            }

            let tenth = (0..30)
                .map(|decision| stratified_exploration_draw(seed, decision, 100_000))
                .collect::<Vec<_>>();
            for cycle in tenth.chunks_exact(10) {
                assert_eq!(cycle.iter().filter(|draw| **draw < 100_000).count(), 1);
            }
        }
    }

    #[test]
    fn an_untrained_catalog_bootstraps_without_fabricating_q() {
        let wait = descriptor("wait", OptionType::Neutral);
        let ranking = LiveTacticRanking {
            learner_snapshot_sha256: Digest([1; 32]),
            action_universe_sha256: Digest([2; 32]),
            choices: vec![choice(wait.clone())],
            values: AvailableOptionRanking {
                ranked: Vec::new(),
                unsupported: vec![wait.clone()],
            },
        };
        let selected = choose_tactic(&ranking, 0, TacticExplorationConfig::default()).unwrap();
        assert_eq!(selected.descriptor, wait);
        assert_eq!(selected.reason, TacticSelectionReason::UnsupportedBootstrap);
    }

    #[test]
    fn nonpositive_known_values_bootstrap_an_unsupported_tactic() {
        let wait = descriptor("wait", OptionType::Neutral);
        let move_forward = descriptor("move", OptionType::Move);
        let ranking = LiveTacticRanking {
            learner_snapshot_sha256: Digest([1; 32]),
            action_universe_sha256: Digest([2; 32]),
            choices: vec![choice(move_forward.clone()), choice(wait.clone())],
            values: AvailableOptionRanking {
                ranked: vec![RankedOption {
                    action_id: 0,
                    descriptor: wait,
                    mean_q: -0.01,
                    ensemble_variance: 0.0,
                }],
                unsupported: vec![move_forward.clone()],
            },
        };
        let selected = choose_tactic(
            &ranking,
            0,
            TacticExplorationConfig {
                seed: 7,
                epsilon_per_million: 0,
            },
        )
        .unwrap();
        assert_eq!(selected.descriptor, move_forward);
        assert_eq!(selected.reason, TacticSelectionReason::UnsupportedBootstrap);
    }

    #[test]
    fn epsilon_exploration_prioritizes_untried_tactics() {
        let known = descriptor("known", OptionType::Neutral);
        let fresh = descriptor("fresh", OptionType::Move);
        let ranking = LiveTacticRanking {
            learner_snapshot_sha256: Digest([3; 32]),
            action_universe_sha256: Digest([4; 32]),
            choices: vec![choice(fresh.clone()), choice(known.clone())],
            values: AvailableOptionRanking {
                ranked: vec![RankedOption {
                    action_id: 1,
                    descriptor: known,
                    mean_q: 5.0,
                    ensemble_variance: 0.0,
                }],
                unsupported: vec![fresh.clone()],
            },
        };
        for seed in 0..16 {
            let selected = choose_tactic(
                &ranking,
                0,
                TacticExplorationConfig {
                    seed,
                    epsilon_per_million: EPSILON_SCALE,
                },
            )
            .unwrap();
            assert_eq!(selected.descriptor, fresh);
            assert_eq!(selected.reason, TacticSelectionReason::Epsilon);
        }
    }

    #[test]
    fn unsupported_navigation_probes_are_covered_before_short_controls() {
        let known = descriptor("known", OptionType::Neutral);
        let mut directional = descriptor("directional", OptionType::MaintainHeading);
        directional
            .parameters
            .insert("heading_radians".into(), OptionParameter::F32Bits(0));
        directional
            .parameters
            .insert("magnitude".into(), OptionParameter::Unsigned(127));
        directional
            .parameters
            .insert("maximum_ticks".into(), OptionParameter::Unsigned(16));
        let mut short = descriptor("short", OptionType::MaintainHeading);
        short
            .parameters
            .insert("heading_radians".into(), OptionParameter::F32Bits(0));
        short
            .parameters
            .insert("magnitude".into(), OptionParameter::Unsigned(80));
        short
            .parameters
            .insert("maximum_ticks".into(), OptionParameter::Unsigned(4));
        let mut curve = descriptor("curve", OptionType::Bezier);
        curve
            .parameters
            .insert("control".into(), OptionParameter::Text("symmetric".into()));
        let mut spatial = descriptor("spatial", OptionType::Move);
        spatial.parameters.insert(
            "coordinate".into(),
            OptionParameter::Vec3F32Bits([1.0_f32.to_bits(), 2.0_f32.to_bits(), 3.0_f32.to_bits()]),
        );
        let ranking = LiveTacticRanking {
            learner_snapshot_sha256: Digest([5; 32]),
            action_universe_sha256: Digest([6; 32]),
            choices: vec![
                choice(directional.clone()),
                choice(known.clone()),
                choice(curve.clone()),
                choice(short.clone()),
                choice(spatial.clone()),
            ],
            values: AvailableOptionRanking {
                ranked: vec![RankedOption {
                    action_id: 1,
                    descriptor: known,
                    mean_q: 5.0,
                    ensemble_variance: 0.0,
                }],
                unsupported: vec![curve.clone(), directional.clone(), short, spatial.clone()],
            },
        };
        let mut selected_ids = std::collections::BTreeSet::new();
        for seed in 0..64 {
            let selected = choose_tactic(
                &ranking,
                0,
                TacticExplorationConfig {
                    seed,
                    epsilon_per_million: EPSILON_SCALE,
                },
            )
            .unwrap();
            assert!(
                selected.descriptor == spatial
                    || selected.descriptor == directional
                    || selected.descriptor == curve,
                "short control was incorrectly prioritized"
            );
            selected_ids.insert(selected.descriptor.option_id);
            assert_eq!(selected.reason, TacticSelectionReason::Epsilon);
        }
        assert_eq!(
            selected_ids,
            std::collections::BTreeSet::from([
                "curve".into(),
                "directional".into(),
                "spatial".into(),
            ])
        );
    }

    #[test]
    fn epsilon_exploration_covers_actions_untried_in_the_current_state_cell() {
        let globally_best = descriptor("globally-best", OptionType::Move);
        let locally_untried = descriptor("locally-untried", OptionType::Bezier);
        let ranking = LiveTacticRanking {
            learner_snapshot_sha256: Digest([7; 32]),
            action_universe_sha256: Digest([8; 32]),
            choices: vec![
                choice(globally_best.clone()),
                choice(locally_untried.clone()),
            ],
            values: AvailableOptionRanking {
                ranked: vec![
                    RankedOption {
                        action_id: 0,
                        descriptor: globally_best,
                        mean_q: 5.0,
                        ensemble_variance: 0.0,
                    },
                    RankedOption {
                        action_id: 1,
                        descriptor: locally_untried.clone(),
                        mean_q: 1.0,
                        ensemble_variance: 0.0,
                    },
                ],
                unsupported: Vec::new(),
            },
        };
        let selected = choose_tactic_with_state_untried(
            &ranking,
            0,
            TacticExplorationConfig {
                seed: 11,
                epsilon_per_million: EPSILON_SCALE,
            },
            std::slice::from_ref(&locally_untried),
        )
        .unwrap();
        assert_eq!(selected.descriptor, locally_untried);
        assert_eq!(selected.reason, TacticSelectionReason::Epsilon);
    }

    #[test]
    fn bounded_coordinate_sequences_are_tried_before_atomic_navigation_probes() {
        let mut route = descriptor(
            "route",
            OptionType::Custom("seek_coordinate_sequence".into()),
        );
        route
            .parameters
            .insert("coordinates".into(), OptionParameter::Text("[]".into()));
        let mut coordinate = descriptor("coordinate", OptionType::Move);
        coordinate.parameters.insert(
            "coordinate".into(),
            OptionParameter::Vec3F32Bits([0.0_f32.to_bits(); 3]),
        );
        let unsupported = [coordinate, route.clone()];
        let prioritized = prioritized_unsupported(&unsupported, false);
        assert_eq!(prioritized, vec![&route]);
    }

    #[test]
    fn layered_goal_route_compositions_remain_in_route_exploration() {
        let mut route = descriptor(
            "goal.seek.route.00",
            OptionType::Custom("seek_coordinate_sequence".into()),
        );
        route
            .parameters
            .insert("coordinates".into(), OptionParameter::Text("[]".into()));
        let rolling_route = descriptor(
            "goal.seek.route.00.roll.period.20.phase.00",
            OptionType::Custom("reactive_controller".into()),
        );
        let mut coordinate = descriptor("coordinate", OptionType::Move);
        coordinate.parameters.insert(
            "coordinate".into(),
            OptionParameter::Vec3F32Bits([0.0_f32.to_bits(); 3]),
        );
        let unsupported = [coordinate, rolling_route.clone(), route.clone()];

        let prioritized = prioritized_unsupported(&unsupported, false);

        assert_eq!(prioritized, vec![&rolling_route, &route]);
    }

    #[test]
    fn terminal_cost_refinement_preserves_control_and_selects_nearest_untried_variant() {
        fn rolling_route(period: u64) -> OptionActionDescriptor {
            let mut route = descriptor(
                &format!("goal.seek.route.00.roll.period.{period:02}.phase.00"),
                OptionType::Custom("reactive_controller".into()),
            );
            route.parameters.insert(
                "program_sha256".into(),
                OptionParameter::Digest(Digest([period as u8; 32])),
            );
            route.parameters.insert(
                "controller_base_sha256".into(),
                OptionParameter::Digest(Digest([9; 32])),
            );
            route
                .parameters
                .insert("duration_ticks".into(), OptionParameter::Unsigned(160));
            route
                .parameters
                .insert("button_pulse_mask".into(), OptionParameter::Unsigned(8));
            route.parameters.insert(
                "button_pulse_period_ticks".into(),
                OptionParameter::Unsigned(period),
            );
            route.parameters.insert(
                "button_pulse_phase_tick".into(),
                OptionParameter::Unsigned(0),
            );
            route
        }

        let incumbent = rolling_route(22);
        let period_20 = rolling_route(20);
        let period_24 = rolling_route(24);
        let mut other_path = rolling_route(22);
        other_path.option_id = "goal.seek.route.01.roll.period.22.phase.00".into();
        other_path.parameters.insert(
            "controller_base_sha256".into(),
            OptionParameter::Digest(Digest([10; 32])),
        );
        let escape = descriptor("interact", OptionType::Interact);
        let ranking = LiveTacticRanking {
            learner_snapshot_sha256: Digest([31; 32]),
            action_universe_sha256: Digest([32; 32]),
            choices: vec![
                choice(incumbent.clone()),
                choice(period_20.clone()),
                choice(period_24.clone()),
                choice(other_path.clone()),
                choice(escape.clone()),
            ],
            values: AvailableOptionRanking {
                ranked: vec![RankedOption {
                    action_id: 0,
                    descriptor: incumbent.clone(),
                    mean_q: 98.5,
                    ensemble_variance: 0.0,
                }],
                unsupported: vec![
                    period_20.clone(),
                    period_24.clone(),
                    other_path.clone(),
                    escape.clone(),
                ],
            },
        };
        let mut proposals = choose_tactic_batch_with_state_untried(
            &ranking,
            4,
            TacticExplorationConfig {
                seed: 7,
                epsilon_per_million: 0,
            },
            &[period_20.clone(), period_24, other_path, escape],
            1,
        )
        .unwrap();
        let mut coverage = proposals[0].clone();
        coverage.descriptor = descriptor("interact", OptionType::Interact);
        coverage.reason = TacticSelectionReason::BatchCoverage;
        proposals.push(coverage);

        ensure_terminal_cost_refinement(
            &ranking,
            &[period_20.clone(), rolling_route(24)],
            Some(&incumbent),
            3,
            &mut proposals,
        )
        .unwrap();

        assert_eq!(proposals.len(), 3);
        assert_eq!(proposals[0].descriptor, incumbent);
        assert_eq!(proposals[1].descriptor, period_20);
        assert_eq!(
            proposals[1].reason,
            TacticSelectionReason::TerminalCostRefinement
        );
    }

    #[test]
    fn escape_actions_are_tried_before_redundant_navigation_probes() {
        let roll = descriptor("roll", OptionType::Roll);
        let interact = descriptor("interact", OptionType::Interact);
        let mut coordinate = descriptor("coordinate", OptionType::Move);
        coordinate.parameters.insert(
            "coordinate".into(),
            OptionParameter::Vec3F32Bits([0.0_f32.to_bits(); 3]),
        );
        let unsupported = [coordinate, interact.clone(), roll.clone()];

        let prioritized = prioritized_unsupported(&unsupported, false);

        assert_eq!(prioritized, vec![&interact, &roll]);
    }

    #[test]
    fn supported_routes_try_semantic_escapes_before_roll_variants() {
        let attack = descriptor("attack", OptionType::Attack);
        let interact = descriptor("interact", OptionType::Interact);
        let roll_short = descriptor("roll-short", OptionType::Roll);
        let roll_long = descriptor("roll-long", OptionType::Roll);
        let unsupported = [roll_short, attack.clone(), roll_long, interact.clone()];

        let prioritized = prioritized_unsupported(&unsupported, true);

        assert_eq!(prioritized, vec![&attack, &interact]);
    }

    #[test]
    fn supported_composite_navigation_makes_new_cells_try_escape_actions_first() {
        let mut supported_route = descriptor(
            "supported-route",
            OptionType::Custom("seek_coordinate_sequence".into()),
        );
        supported_route
            .parameters
            .insert("coordinates".into(), OptionParameter::Text("[]".into()));
        let mut fresh_route = supported_route.clone();
        fresh_route.option_id = "fresh-route".into();
        let roll = descriptor("roll", OptionType::Roll);
        let ranking = LiveTacticRanking {
            learner_snapshot_sha256: Digest([9; 32]),
            action_universe_sha256: Digest([10; 32]),
            choices: vec![
                choice(supported_route.clone()),
                choice(fresh_route.clone()),
                choice(roll.clone()),
            ],
            values: AvailableOptionRanking {
                ranked: vec![RankedOption {
                    action_id: 0,
                    descriptor: supported_route,
                    mean_q: 1.0,
                    ensemble_variance: 0.0,
                }],
                unsupported: vec![fresh_route.clone(), roll.clone()],
            },
        };

        let selected = choose_tactic_with_state_untried(
            &ranking,
            0,
            TacticExplorationConfig {
                seed: 1,
                epsilon_per_million: EPSILON_SCALE,
            },
            &[fresh_route, roll.clone()],
        )
        .unwrap();

        assert_eq!(selected.descriptor, roll);
        assert_eq!(selected.reason, TacticSelectionReason::Epsilon);
    }

    #[test]
    fn proposal_batch_preserves_primary_and_prioritizes_distinct_types() {
        let move_a = descriptor("move-a", OptionType::Move);
        let move_b = descriptor("move-b", OptionType::Move);
        let roll = descriptor("roll", OptionType::Roll);
        let wait = descriptor("wait", OptionType::Neutral);
        let ranking = LiveTacticRanking {
            learner_snapshot_sha256: Digest([11; 32]),
            action_universe_sha256: Digest([12; 32]),
            choices: vec![
                choice(move_b.clone()),
                choice(wait.clone()),
                choice(move_a.clone()),
                choice(roll.clone()),
            ],
            values: AvailableOptionRanking {
                ranked: vec![RankedOption {
                    action_id: 0,
                    descriptor: move_a.clone(),
                    mean_q: 5.0,
                    ensemble_variance: 0.0,
                }],
                unsupported: vec![move_b, roll, wait],
            },
        };
        let config = TacticExplorationConfig {
            seed: 42,
            epsilon_per_million: 0,
        };

        let primary = choose_tactic(&ranking, 3, config).unwrap();
        let first = choose_tactic_batch_with_state_untried(&ranking, 3, config, &[], 3).unwrap();
        let second = choose_tactic_batch_with_state_untried(&ranking, 3, config, &[], 3).unwrap();

        assert_eq!(first, second);
        assert_eq!(first[0], primary);
        assert_eq!(first.len(), 3);
        assert_eq!(first[1].reason, TacticSelectionReason::BatchCoverage);
        assert_eq!(first[2].reason, TacticSelectionReason::BatchCoverage);
        assert!(first[1].descriptor.option_type != OptionType::Move);
        assert!(first[2].descriptor.option_type != OptionType::Move);
        assert!(first[1].descriptor.option_type != first[2].descriptor.option_type);
    }

    #[test]
    fn proposal_batch_has_separate_uncertainty_value_and_coverage_lanes() {
        let greedy = descriptor("greedy", OptionType::Move);
        let uncertain = descriptor("uncertain", OptionType::Bezier);
        let valuable = descriptor("valuable", OptionType::Roll);
        let fresh = descriptor("fresh", OptionType::Interact);
        let ranking = LiveTacticRanking {
            learner_snapshot_sha256: Digest([15; 32]),
            action_universe_sha256: Digest([16; 32]),
            choices: vec![
                choice(fresh.clone()),
                choice(valuable.clone()),
                choice(greedy.clone()),
                choice(uncertain.clone()),
            ],
            values: AvailableOptionRanking {
                ranked: vec![
                    RankedOption {
                        action_id: 0,
                        descriptor: greedy.clone(),
                        mean_q: 9.0,
                        ensemble_variance: 0.1,
                    },
                    RankedOption {
                        action_id: 1,
                        descriptor: valuable.clone(),
                        mean_q: 7.0,
                        ensemble_variance: 0.2,
                    },
                    RankedOption {
                        action_id: 2,
                        descriptor: uncertain.clone(),
                        mean_q: 1.0,
                        ensemble_variance: 8.0,
                    },
                ],
                unsupported: vec![fresh.clone()],
            },
        };
        let batch = choose_tactic_batch_with_state_untried(
            &ranking,
            0,
            TacticExplorationConfig {
                seed: 19,
                epsilon_per_million: 0,
            },
            &[uncertain.clone(), valuable.clone(), fresh.clone()],
            4,
        )
        .unwrap();

        assert_eq!(batch[0].descriptor, greedy);
        assert_eq!(batch[1].descriptor, uncertain);
        assert_eq!(batch[1].reason, TacticSelectionReason::BatchUncertainty);
        assert_eq!(batch[2].descriptor, valuable);
        assert_eq!(batch[2].reason, TacticSelectionReason::BatchValue);
        assert_eq!(batch[3].descriptor, fresh);
        assert_eq!(batch[3].reason, TacticSelectionReason::BatchCoverage);
    }

    #[test]
    fn proposal_batch_covers_an_untried_action_before_remeasuring_ranked_actions() {
        let greedy = descriptor("greedy", OptionType::Move);
        let uncertain = descriptor("uncertain", OptionType::Bezier);
        let fresh = descriptor("fresh", OptionType::Interact);
        let ranking = LiveTacticRanking {
            learner_snapshot_sha256: Digest([17; 32]),
            action_universe_sha256: Digest([18; 32]),
            choices: vec![
                choice(fresh.clone()),
                choice(greedy.clone()),
                choice(uncertain.clone()),
            ],
            values: AvailableOptionRanking {
                ranked: vec![
                    RankedOption {
                        action_id: 0,
                        descriptor: greedy.clone(),
                        mean_q: 9.0,
                        ensemble_variance: 0.1,
                    },
                    RankedOption {
                        action_id: 1,
                        descriptor: uncertain,
                        mean_q: 1.0,
                        ensemble_variance: 8.0,
                    },
                ],
                unsupported: vec![fresh.clone()],
            },
        };

        let batch = choose_tactic_batch_with_state_untried(
            &ranking,
            0,
            TacticExplorationConfig {
                seed: 23,
                epsilon_per_million: 0,
            },
            std::slice::from_ref(&fresh),
            2,
        )
        .unwrap();

        assert_eq!(batch[0].descriptor, greedy);
        assert_eq!(batch[1].descriptor, fresh);
        assert_eq!(batch[1].reason, TacticSelectionReason::BatchCoverage);
    }

    #[test]
    fn exploratory_batch_keeps_one_greedy_control() {
        let greedy = descriptor("greedy", OptionType::Move);
        let fresh = descriptor("fresh", OptionType::Interact);
        let ranking = LiveTacticRanking {
            learner_snapshot_sha256: Digest([19; 32]),
            action_universe_sha256: Digest([20; 32]),
            choices: vec![choice(fresh.clone()), choice(greedy.clone())],
            values: AvailableOptionRanking {
                ranked: vec![RankedOption {
                    action_id: 0,
                    descriptor: greedy.clone(),
                    mean_q: 9.0,
                    ensemble_variance: 0.1,
                }],
                unsupported: vec![fresh.clone()],
            },
        };

        let batch = choose_tactic_batch_with_state_untried(
            &ranking,
            0,
            TacticExplorationConfig {
                seed: 29,
                epsilon_per_million: EPSILON_SCALE,
            },
            std::slice::from_ref(&fresh),
            2,
        )
        .unwrap();

        assert_eq!(batch[0].descriptor, fresh);
        assert_eq!(batch[0].reason, TacticSelectionReason::Epsilon);
        assert_eq!(batch[1].descriptor, greedy);
        assert_eq!(batch[1].reason, TacticSelectionReason::BatchValue);
    }

    #[test]
    fn proposal_batch_rejects_zero_capacity_and_never_duplicates_actions() {
        let move_a = descriptor("move-a", OptionType::Move);
        let move_b = descriptor("move-b", OptionType::Move);
        let ranking = LiveTacticRanking {
            learner_snapshot_sha256: Digest([13; 32]),
            action_universe_sha256: Digest([14; 32]),
            choices: vec![choice(move_b.clone()), choice(move_a.clone())],
            values: AvailableOptionRanking {
                ranked: vec![RankedOption {
                    action_id: 0,
                    descriptor: move_a,
                    mean_q: 1.0,
                    ensemble_variance: 0.0,
                }],
                unsupported: vec![move_b],
            },
        };
        let config = TacticExplorationConfig {
            seed: 7,
            epsilon_per_million: 0,
        };

        assert_eq!(
            choose_tactic_batch_with_state_untried(&ranking, 0, config, &[], 0),
            Err(TacticExplorationError::InvalidInput)
        );
        let batch =
            choose_tactic_batch_with_state_untried(&ranking, 0, config, &[], usize::MAX).unwrap();
        assert_eq!(batch.len(), 2);
        assert_ne!(batch[0].descriptor, batch[1].descriptor);
    }

    #[test]
    fn equal_budget_baselines_ignore_learned_values_and_remain_seeded() {
        let choices = [
            descriptor("move", OptionType::Move),
            descriptor("roll", OptionType::Roll),
            descriptor("wait", OptionType::Neutral),
            descriptor("interact", OptionType::Interact),
        ];
        let mut ranking = LiveTacticRanking {
            learner_snapshot_sha256: Digest([21; 32]),
            action_universe_sha256: Digest([22; 32]),
            choices: choices.iter().cloned().map(choice).collect(),
            values: AvailableOptionRanking {
                ranked: vec![RankedOption {
                    action_id: 0,
                    descriptor: choices[0].clone(),
                    mean_q: 100.0,
                    ensemble_variance: 0.0,
                }],
                unsupported: choices[1..].to_vec(),
            },
        };
        let config = TacticExplorationConfig {
            seed: 104_729,
            epsilon_per_million: 350_000,
        };
        let random = choose_tactic_batch_for_policy(
            &ranking,
            7,
            config,
            &choices,
            3,
            TacticProposalPolicy::RandomValid,
        )
        .unwrap();
        let structured = choose_tactic_batch_for_policy(
            &ranking,
            7,
            config,
            &choices,
            3,
            TacticProposalPolicy::StructuredNonLearning,
        )
        .unwrap();
        ranking.values.ranked[0].mean_q = -100.0;
        assert_eq!(
            random,
            choose_tactic_batch_for_policy(
                &ranking,
                7,
                config,
                &choices,
                3,
                TacticProposalPolicy::RandomValid,
            )
            .unwrap()
        );
        assert_eq!(
            structured,
            choose_tactic_batch_for_policy(
                &ranking,
                7,
                config,
                &choices,
                3,
                TacticProposalPolicy::StructuredNonLearning,
            )
            .unwrap()
        );
        assert!(
            random
                .iter()
                .all(|proposal| proposal.reason == TacticSelectionReason::RandomBaseline)
        );
        assert!(
            structured
                .iter()
                .all(|proposal| { proposal.reason == TacticSelectionReason::StructuredBaseline })
        );
        assert_eq!(
            random
                .iter()
                .map(|proposal| proposal.descriptor.option_id.as_str())
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            3
        );
    }

    #[test]
    fn random_valid_baseline_obeys_the_live_applicability_mask() {
        let available = descriptor("available", OptionType::Move);
        let unavailable = descriptor("unavailable", OptionType::Roll);
        let ranking = LiveTacticRanking {
            learner_snapshot_sha256: Digest([23; 32]),
            action_universe_sha256: Digest([24; 32]),
            choices: vec![
                choice(available.clone()),
                LearnerActionMaskEntry {
                    applicable: false,
                    ..choice(unavailable)
                },
            ],
            values: AvailableOptionRanking {
                ranked: Vec::new(),
                unsupported: vec![available.clone()],
            },
        };
        let selected = choose_tactic_batch_for_policy(
            &ranking,
            0,
            TacticExplorationConfig {
                seed: 104_729,
                epsilon_per_million: 0,
            },
            std::slice::from_ref(&available),
            4,
            TacticProposalPolicy::RandomValid,
        )
        .unwrap();

        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].descriptor, available);
    }
}
