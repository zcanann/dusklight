use super::{
    MAX_GENERALIZED_VALUE_ACQUISITION_RANKS, SelectedTactic, TacticExplorationError,
    TacticSelectionReason,
};
use crate::option_values::OptionActionDescriptor;
use dusklight_control::option_execution::{OptionParameter, OptionType};

const MAX_TERMINAL_SUPPORT_FACTORS_PER_BATCH: usize = 4;

/// Reserve factor-diverse behavior-transfer probes on the terminal-support lane.
///
/// A demonstration can establish useful movement intent without containing
/// every executable realization of that intent. The ordinary generalized
/// acquisition reserves only one descriptor, which makes a high-ranked plain
/// movement action crowd out currently available prompted buttons or longer
/// movement. This lane first keeps the highest-ranked descriptor for each
/// independent button mask, then fills remaining slots from distinct
/// `(type, logarithmic duration)` blocks. Their measured outcomes still
/// determine value; the demonstration supplies action similarity, not reward.
pub fn ensure_terminal_support_factor_acquisitions(
    ranked_applicable: &[OptionActionDescriptor],
    maximum_proposals: usize,
    proposals: &mut Vec<SelectedTactic>,
) -> Result<(), TacticExplorationError> {
    if maximum_proposals <= 1 || proposals.is_empty() || proposals.len() > maximum_proposals {
        return if !proposals.is_empty() && proposals.len() <= maximum_proposals {
            Ok(())
        } else {
            Err(TacticExplorationError::InvalidInput)
        };
    }
    let mut ranked = interleave_ranked_action_factors(ranked_applicable)
        .into_iter()
        .take(MAX_GENERALIZED_VALUE_ACQUISITION_RANKS)
        .collect::<Vec<_>>();
    if ranked.is_empty() {
        return Ok(());
    }

    let template = proposals[0].clone();
    let original = std::mem::take(proposals);
    let mut rebuilt = Vec::with_capacity(maximum_proposals);
    rebuilt.push(original[0].clone());
    let mut represented_factors = vec![action_factor_block(&original[0].descriptor)];
    let mut represented_button_masks = vec![action_button_mask(&original[0].descriptor)];
    let factorized_limit = maximum_proposals.min(MAX_TERMINAL_SUPPORT_FACTORS_PER_BATCH);
    for descriptor in &ranked {
        let button_mask = action_button_mask(descriptor);
        if represented_button_masks.contains(&button_mask) {
            continue;
        }
        push_generalized_factor_acquisition(
            descriptor,
            &template,
            &mut rebuilt,
            &mut represented_factors,
        );
        represented_button_masks.push(button_mask);
        if rebuilt.len() == factorized_limit {
            break;
        }
    }
    let primary_duration_band = action_duration_band(&original[0].descriptor);
    if rebuilt.len() < factorized_limit
        && let Some(index) = ranked
            .iter()
            .position(|descriptor| action_duration_band(descriptor) != primary_duration_band)
    {
        // Duration is an independent learned action factor. Reserve the best
        // ranked alternative horizon before filling coarse behavior types, or
        // a short forced-primary action can consume every slot with short
        // siblings before a longer terminal-capable probe is considered.
        let duration_probe = ranked.remove(index);
        ranked.insert(0, duration_probe);
    }
    for descriptor in ranked {
        if rebuilt.len() == factorized_limit {
            break;
        }
        let factor = action_factor_block(descriptor);
        if represented_factors.contains(&factor) {
            continue;
        }
        push_generalized_factor_acquisition(
            descriptor,
            &template,
            &mut rebuilt,
            &mut represented_factors,
        );
    }
    for proposal in original.into_iter().skip(1) {
        if rebuilt.len() == maximum_proposals {
            break;
        }
        if !rebuilt
            .iter()
            .any(|existing| existing.descriptor == proposal.descriptor)
        {
            rebuilt.push(proposal);
        }
    }
    *proposals = rebuilt;
    Ok(())
}

fn push_generalized_factor_acquisition(
    descriptor: &OptionActionDescriptor,
    template: &SelectedTactic,
    proposals: &mut Vec<SelectedTactic>,
    represented_factors: &mut Vec<(OptionType, u32)>,
) {
    if let Some(existing) = proposals
        .iter_mut()
        .find(|proposal| proposal.descriptor == *descriptor)
    {
        if existing.reason != TacticSelectionReason::Epsilon {
            existing.reason = TacticSelectionReason::GeneralizedValue;
        }
        return;
    }
    let mut acquisition = template.clone();
    acquisition.descriptor = descriptor.clone();
    acquisition.reason = TacticSelectionReason::GeneralizedValue;
    represented_factors.push(action_factor_block(descriptor));
    proposals.push(acquisition);
}

fn interleave_ranked_action_factors(
    ranked: &[OptionActionDescriptor],
) -> Vec<&OptionActionDescriptor> {
    let mut groups =
        Vec::<((OptionType, u32), Vec<&OptionActionDescriptor>)>::new();
    for descriptor in ranked {
        let factor = action_factor_block(descriptor);
        if let Some((_, members)) = groups
            .iter_mut()
            .find(|(existing, _)| existing == &factor)
        {
            members.push(descriptor);
        } else {
            groups.push((factor, vec![descriptor]));
        }
    }
    let maximum_group = groups
        .iter()
        .map(|(_, members)| members.len())
        .max()
        .unwrap_or(0);
    let mut interleaved =
        Vec::with_capacity(groups.iter().map(|(_, members)| members.len()).sum());
    for rank_within_factor in 0..maximum_group {
        for (_, members) in &groups {
            if let Some(descriptor) = members.get(rank_within_factor) {
                interleaved.push(*descriptor);
            }
        }
    }
    interleaved
}

fn action_factor_block(descriptor: &OptionActionDescriptor) -> (OptionType, u32) {
    (
        descriptor.option_type.clone(),
        action_duration_band(descriptor),
    )
}

fn action_duration_band(descriptor: &OptionActionDescriptor) -> u32 {
    let duration = unsigned_parameter(descriptor, &["duration_ticks", "maximum_ticks"])
        .or_else(|| {
            unsigned_parameter(descriptor, &["recovery_frames"])
                .and_then(|frames| frames.checked_add(1))
        })
        .unwrap_or(1);
    u64::BITS - 1 - duration.max(1).leading_zeros()
}

pub(super) fn action_button_mask(descriptor: &OptionActionDescriptor) -> u16 {
    unsigned_parameter(
        descriptor,
        &["command_button_mask", "button_pulse_mask", "button_mask"],
    )
    .unwrap_or_else(|| u64::from(descriptor.option_type == OptionType::Roll) * 0x0100)
        as u16
}

fn unsigned_parameter(descriptor: &OptionActionDescriptor, names: &[&str]) -> Option<u64> {
    names.iter().find_map(|name| {
        descriptor.parameters.get(*name).and_then(|value| match value {
            OptionParameter::Unsigned(value) => Some(*value),
            _ => None,
        })
    })
}
