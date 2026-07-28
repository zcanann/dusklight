use super::{SelectedTactic, TacticExplorationError, TacticSelectionReason};
use crate::generalized_tactic_value::{
    GENERALIZED_TACTIC_ACTION_FEATURE_WIDTH, GeneralizedTacticContext, encode_action,
};
use crate::option_values::OptionActionDescriptor;

// The first 29 action features are the option-type one-hot encoding. Existing
// batch acquisition already spreads proposals across option types. This lane
// covers the executable factors within those types: duration, emitted heading,
// target/path geometry, stick magnitude, and prompted-button timing.
const EXECUTABLE_FACTOR_START: usize = 29;
const MINIMUM_FACTOR_RANGE: f32 = 1.0e-6;

struct EncodedAction<'a> {
    descriptor: &'a OptionActionDescriptor,
    factors: [f32; GENERALIZED_TACTIC_ACTION_FEATURE_WIDTH],
}

/// Reserve one learned-policy acquisition slot for the applicable controller
/// whose executable factors are farthest from the rest of the current batch.
///
/// Factor ranges come from the live applicable action universe at this exact
/// state. The selector therefore has no route, option-ID, or authored family
/// knowledge. State-local untried actions are preferred when any exist.
pub fn ensure_action_factor_coverage(
    context: &GeneralizedTacticContext,
    applicable: &[OptionActionDescriptor],
    state_untried: &[OptionActionDescriptor],
    maximum_proposals: usize,
    proposals: &mut Vec<SelectedTactic>,
) -> Result<(), TacticExplorationError> {
    if maximum_proposals <= 1
        || proposals.is_empty()
        || proposals.len() > maximum_proposals
        || applicable.is_empty()
        || state_untried.iter().enumerate().any(|(index, descriptor)| {
            !applicable.contains(descriptor) || state_untried[..index].contains(descriptor)
        })
    {
        return if maximum_proposals <= 1
            && !proposals.is_empty()
            && proposals.len() <= maximum_proposals
        {
            Ok(())
        } else {
            Err(TacticExplorationError::InvalidInput)
        };
    }

    let encoded = applicable
        .iter()
        .map(|descriptor| {
            encode_action(context, descriptor)
                .map(|factors| EncodedAction {
                    descriptor,
                    factors,
                })
                .map_err(|_| TacticExplorationError::InvalidInput)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let (minimum, range) = factor_ranges(&encoded);
    let proposed_factors = proposals
        .iter()
        .map(|proposal| {
            encoded
                .iter()
                .find(|action| action.descriptor == &proposal.descriptor)
                .map(|action| &action.factors)
                .ok_or(TacticExplorationError::DetachedRanking)
        })
        .collect::<Result<Vec<_>, _>>()?;

    let mut candidates = encoded
        .iter()
        .filter(|action| {
            !proposals
                .iter()
                .any(|proposal| proposal.descriptor == *action.descriptor)
        })
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return Ok(());
    }
    if candidates
        .iter()
        .any(|candidate| state_untried.contains(candidate.descriptor))
    {
        candidates.retain(|candidate| state_untried.contains(candidate.descriptor));
    }
    candidates.sort_by(|left, right| {
        let left_distance =
            nearest_factor_distance(&left.factors, &proposed_factors, &minimum, &range);
        let right_distance =
            nearest_factor_distance(&right.factors, &proposed_factors, &minimum, &range);
        right_distance
            .total_cmp(&left_distance)
            .then_with(|| left.descriptor.option_id.cmp(&right.descriptor.option_id))
    });

    let mut coverage = proposals[0].clone();
    coverage.descriptor = candidates[0].descriptor.clone();
    coverage.reason = TacticSelectionReason::BatchCoverage;
    if proposals.len() < maximum_proposals {
        proposals.push(coverage);
        return Ok(());
    }
    let replacement = [
        TacticSelectionReason::BatchCoverage,
        TacticSelectionReason::BatchUncertainty,
        TacticSelectionReason::BatchValue,
    ]
    .into_iter()
    .find_map(|reason| {
        proposals
            .iter()
            .enumerate()
            .rev()
            .find(|(index, proposal)| *index > 0 && proposal.reason == reason)
            .map(|(index, _)| index)
    });
    if let Some(index) = replacement {
        proposals[index] = coverage;
    }
    Ok(())
}

fn factor_ranges(
    actions: &[EncodedAction<'_>],
) -> (
    [f32; GENERALIZED_TACTIC_ACTION_FEATURE_WIDTH],
    [f32; GENERALIZED_TACTIC_ACTION_FEATURE_WIDTH],
) {
    let mut minimum = [f32::INFINITY; GENERALIZED_TACTIC_ACTION_FEATURE_WIDTH];
    let mut maximum = [f32::NEG_INFINITY; GENERALIZED_TACTIC_ACTION_FEATURE_WIDTH];
    for action in actions {
        for index in EXECUTABLE_FACTOR_START..GENERALIZED_TACTIC_ACTION_FEATURE_WIDTH {
            minimum[index] = minimum[index].min(action.factors[index]);
            maximum[index] = maximum[index].max(action.factors[index]);
        }
    }
    let mut range = [0.0; GENERALIZED_TACTIC_ACTION_FEATURE_WIDTH];
    for index in EXECUTABLE_FACTOR_START..GENERALIZED_TACTIC_ACTION_FEATURE_WIDTH {
        range[index] = maximum[index] - minimum[index];
    }
    (minimum, range)
}

fn nearest_factor_distance(
    candidate: &[f32; GENERALIZED_TACTIC_ACTION_FEATURE_WIDTH],
    proposed: &[&[f32; GENERALIZED_TACTIC_ACTION_FEATURE_WIDTH]],
    minimum: &[f32; GENERALIZED_TACTIC_ACTION_FEATURE_WIDTH],
    range: &[f32; GENERALIZED_TACTIC_ACTION_FEATURE_WIDTH],
) -> f32 {
    proposed
        .iter()
        .map(|other| normalized_factor_distance(candidate, other, minimum, range))
        .min_by(f32::total_cmp)
        .unwrap_or(0.0)
}

fn normalized_factor_distance(
    left: &[f32; GENERALIZED_TACTIC_ACTION_FEATURE_WIDTH],
    right: &[f32; GENERALIZED_TACTIC_ACTION_FEATURE_WIDTH],
    minimum: &[f32; GENERALIZED_TACTIC_ACTION_FEATURE_WIDTH],
    range: &[f32; GENERALIZED_TACTIC_ACTION_FEATURE_WIDTH],
) -> f32 {
    let mut squared_distance = 0.0;
    let mut dimensions = 0;
    for index in EXECUTABLE_FACTOR_START..GENERALIZED_TACTIC_ACTION_FEATURE_WIDTH {
        if range[index] > MINIMUM_FACTOR_RANGE {
            let left = (left[index] - minimum[index]) / range[index];
            let right = (right[index] - minimum[index]) / range[index];
            squared_distance += (left - right).powi(2);
            dimensions += 1;
        }
    }
    if dimensions == 0 {
        0.0
    } else {
        squared_distance / dimensions as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact::Digest;
    use crate::option_values::OptionActionDescriptor;
    use dusklight_control::option_execution::{OptionParameter, OptionType};
    use std::collections::BTreeMap;

    fn heading(id: &str, radians: f32, duration: u64) -> OptionActionDescriptor {
        OptionActionDescriptor {
            option_id: id.into(),
            option_type: OptionType::MaintainHeading,
            parameters: BTreeMap::from([
                (
                    "heading_radians".into(),
                    OptionParameter::F32Bits(radians.to_bits()),
                ),
                ("magnitude".into(), OptionParameter::Unsigned(127)),
                ("maximum_ticks".into(), OptionParameter::Unsigned(duration)),
            ]),
        }
    }

    fn proposal(
        descriptor: OptionActionDescriptor,
        reason: TacticSelectionReason,
    ) -> SelectedTactic {
        SelectedTactic {
            schema: super::super::TACTIC_EXPLORATION_SCHEMA_V1.into(),
            learner_snapshot_sha256: Digest([31; 32]),
            decision_index: 7,
            descriptor,
            reason,
            exploration_draw: 0,
        }
    }

    #[test]
    fn executable_coverage_selects_the_farthest_untried_heading() {
        let direct = heading("direct", 0.0, 4);
        let nearby = heading("nearby", 0.25, 4);
        let opposite = heading("opposite", std::f32::consts::PI, 40);
        let applicable = vec![direct.clone(), nearby.clone(), opposite.clone()];
        let mut proposals = vec![proposal(direct, TacticSelectionReason::Greedy)];

        ensure_action_factor_coverage(
            &GeneralizedTacticContext::default(),
            &applicable,
            &[nearby, opposite.clone()],
            2,
            &mut proposals,
        )
        .unwrap();

        assert_eq!(proposals[1].descriptor, opposite);
        assert_eq!(proposals[1].reason, TacticSelectionReason::BatchCoverage);
    }

    #[test]
    fn executable_coverage_prefers_untried_and_preserves_authoritative_slots() {
        let epsilon = heading("epsilon", 0.0, 4);
        let generalized = heading("generalized", 0.25, 4);
        let tried_far = heading("tried-far", std::f32::consts::PI, 40);
        let untried = heading("untried", 0.5, 8);
        let replaceable = heading("replaceable", 0.1, 4);
        let applicable = vec![
            epsilon.clone(),
            generalized.clone(),
            tried_far,
            untried.clone(),
            replaceable.clone(),
        ];
        let mut proposals = vec![
            proposal(epsilon.clone(), TacticSelectionReason::Epsilon),
            proposal(generalized.clone(), TacticSelectionReason::GeneralizedValue),
            proposal(replaceable, TacticSelectionReason::BatchUncertainty),
        ];

        ensure_action_factor_coverage(
            &GeneralizedTacticContext::default(),
            &applicable,
            std::slice::from_ref(&untried),
            3,
            &mut proposals,
        )
        .unwrap();

        assert_eq!(proposals[0].descriptor, epsilon);
        assert_eq!(proposals[0].reason, TacticSelectionReason::Epsilon);
        assert_eq!(proposals[1].descriptor, generalized);
        assert_eq!(proposals[1].reason, TacticSelectionReason::GeneralizedValue);
        assert_eq!(proposals[2].descriptor, untried);
    }

    #[test]
    fn executable_coverage_does_not_evict_a_full_authoritative_batch() {
        let greedy = heading("greedy", 0.0, 4);
        let generalized = heading("generalized", 0.25, 4);
        let refinement = heading("refinement", 0.5, 8);
        let candidate = heading("candidate", std::f32::consts::PI, 40);
        let applicable = vec![
            greedy.clone(),
            generalized.clone(),
            refinement.clone(),
            candidate,
        ];
        let mut proposals = vec![
            proposal(greedy.clone(), TacticSelectionReason::Greedy),
            proposal(generalized.clone(), TacticSelectionReason::GeneralizedValue),
            proposal(
                refinement.clone(),
                TacticSelectionReason::TerminalCostRefinement,
            ),
        ];

        ensure_action_factor_coverage(
            &GeneralizedTacticContext::default(),
            &applicable,
            &[],
            3,
            &mut proposals,
        )
        .unwrap();

        assert_eq!(
            proposals
                .iter()
                .map(|proposal| &proposal.descriptor)
                .collect::<Vec<_>>(),
            vec![&greedy, &generalized, &refinement]
        );
    }
}
