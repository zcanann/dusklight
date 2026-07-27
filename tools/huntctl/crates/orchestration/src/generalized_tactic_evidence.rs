//! Leave-controller-out evidence for the shared tactic outcome model.
//!
//! The input corpora are authenticated native transition envelopes. Each
//! comparison removes every occurrence of both exact action descriptors,
//! refits the shared model, and asks it to rank the unseen pair from their
//! common source state.

use crate::tactic_q_campaign::TacticQTrainingCorpus;
use dusklight_learning::artifact::Digest;
use dusklight_learning::generalized_tactic_value::{
    GeneralizedTacticActionFactors, GeneralizedTacticContext, GeneralizedTacticOutcome,
    GeneralizedTacticValueModel, compare_generalized_tactic_outcomes,
    generalized_tactic_action_factors,
};
use dusklight_learning::option_transition::OptionTransitionSample;
use dusklight_learning::option_values::OptionActionDescriptor;
use serde::Serialize;
use sha2::{Digest as ShaDigest, Sha256};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

pub const GENERALIZED_TACTIC_HELD_OUT_EVIDENCE_SCHEMA_V1: &str =
    "dusklight-generalized-tactic-held-out-evidence/v1";

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GeneralizedTacticHeldOutEvidence {
    pub schema: String,
    pub feature_schema_sha256: Digest,
    pub objective_sha256: Digest,
    pub root_checkpoint_sha256: Digest,
    pub goal_distance_feature: usize,
    pub input_corpora: u64,
    pub authenticated_native_transitions: u64,
    pub unique_native_transitions: u64,
    pub unique_controller_instances: u64,
    pub exact_controller_instances_excluded_per_comparison: u64,
    pub comparisons: Vec<GeneralizedTacticHeldOutComparison>,
    pub passed: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GeneralizedTacticHeldOutComparison {
    pub category: String,
    pub favorable: GeneralizedTacticHeldOutAction,
    pub comparison: GeneralizedTacticHeldOutAction,
    pub training_transitions_after_exclusion: u64,
    pub predicted_first_option_id: String,
    pub passed: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GeneralizedTacticHeldOutAction {
    pub option_id: String,
    pub action_sha256: Digest,
    pub before_state_sha256: Digest,
    pub factors: GeneralizedTacticActionFactors,
    pub actual: GeneralizedTacticOutcome,
    pub predicted: GeneralizedTacticOutcome,
}

#[derive(Clone)]
struct HeldOutAction {
    action_sha256: Digest,
    transition: OptionTransitionSample,
    context: GeneralizedTacticContext,
    factors: GeneralizedTacticActionFactors,
    actual: GeneralizedTacticOutcome,
    geometry_sha256: Digest,
}

pub fn prove_generalized_tactic_held_out_value(
    corpora: &[TacticQTrainingCorpus],
    goal_distance_feature: usize,
) -> Result<GeneralizedTacticHeldOutEvidence, GeneralizedTacticEvidenceError> {
    let first = corpora
        .first()
        .ok_or_else(|| evidence_message("held-out evidence requires native training corpora"))?;
    if corpora.iter().any(|corpus| {
        corpus.feature_schema_sha256 != first.feature_schema_sha256
            || corpus.objective_sha256 != first.objective_sha256
            || corpus.root_checkpoint_sha256 != first.root_checkpoint_sha256
    }) {
        return Err(evidence_message(
            "held-out corpora do not share feature, objective, and root checkpoint identities",
        ));
    }
    let authenticated_native_transitions = corpora
        .iter()
        .map(|corpus| corpus.transitions.len() as u64)
        .sum();
    let mut unique = BTreeMap::<Digest, OptionTransitionSample>::new();
    for corpus in corpora {
        if corpus.transitions.len() != corpus.routes.len()
            || corpus.transitions.len() != corpus.episode_groups.len()
        {
            return Err(evidence_message("held-out corpus shape is detached"));
        }
        for transition in &corpus.transitions {
            transition
                .validate()
                .map_err(|error| evidence_message(error.to_string()))?;
            unique
                .entry(
                    transition
                        .replay_identity_sha256()
                        .map_err(|error| evidence_message(error.to_string()))?,
                )
                .or_insert_with(|| transition.clone());
        }
    }
    let transitions = unique.into_values().collect::<Vec<_>>();
    if transitions.len() < 4 {
        return Err(evidence_message(
            "held-out evidence requires at least four unique native transitions",
        ));
    }

    let mut representatives = BTreeMap::<Digest, HeldOutAction>::new();
    for transition in &transitions {
        let action_sha256 = transition
            .value_sample
            .action
            .content_sha256()
            .map_err(|error| evidence_message(error.to_string()))?;
        let context = GeneralizedTacticContext::from_facts(&transition.before)
            .map_err(|error| evidence_message(error.to_string()))?;
        let factors = generalized_tactic_action_factors(&context, &transition.value_sample.action)
            .map_err(|error| evidence_message(error.to_string()))?;
        let actual = GeneralizedTacticOutcome::from_transition(transition, goal_distance_feature)
            .map_err(|error| evidence_message(error.to_string()))?;
        let candidate = HeldOutAction {
            action_sha256,
            transition: transition.clone(),
            context,
            factors,
            actual,
            geometry_sha256: action_geometry_sha256(&transition.value_sample.action)?,
        };
        match representatives.get(&action_sha256) {
            Some(existing)
                if compare_generalized_tactic_outcomes(&existing.actual, &candidate.actual)
                    != Ordering::Less => {}
            _ => {
                representatives.insert(action_sha256, candidate);
            }
        }
    }
    let actions = representatives.into_values().collect::<Vec<_>>();
    let pairs = [
        rolling_pair(&actions)?,
        straight_pair(&actions)?,
        collision_pair(&actions)?,
    ];
    let mut comparisons = Vec::with_capacity(pairs.len());
    for (category, favorable, comparison) in pairs {
        comparisons.push(evaluate_pair(
            category,
            favorable,
            comparison,
            &transitions,
            goal_distance_feature,
        )?);
    }
    let passed = comparisons.len() == 3 && comparisons.iter().all(|comparison| comparison.passed);
    Ok(GeneralizedTacticHeldOutEvidence {
        schema: GENERALIZED_TACTIC_HELD_OUT_EVIDENCE_SCHEMA_V1.into(),
        feature_schema_sha256: first.feature_schema_sha256,
        objective_sha256: first.objective_sha256,
        root_checkpoint_sha256: first.root_checkpoint_sha256,
        goal_distance_feature,
        input_corpora: corpora.len() as u64,
        authenticated_native_transitions,
        unique_native_transitions: transitions.len() as u64,
        unique_controller_instances: actions.len() as u64,
        exact_controller_instances_excluded_per_comparison: 2,
        comparisons,
        passed,
    })
}

fn rolling_pair(
    actions: &[HeldOutAction],
) -> Result<(&'static str, &HeldOutAction, &HeldOutAction), GeneralizedTacticEvidenceError> {
    let mut candidates = Vec::new();
    for favorable in actions.iter().filter(|action| action.factors.rolling) {
        for comparison in actions.iter().filter(|action| !action.factors.rolling) {
            let distance = factor_distance(favorable, comparison);
            if comparable_source(favorable, comparison)
                && (favorable.geometry_sha256 == comparison.geometry_sha256 || distance <= 0.75)
                && compare_generalized_tactic_outcomes(&favorable.actual, &comparison.actual)
                    == Ordering::Greater
            {
                candidates.push((distance, favorable, comparison));
            }
        }
    }
    candidates.sort_by(|left, right| {
        left.0
            .total_cmp(&right.0)
            .then_with(|| right.1.actual.terminal.total_cmp(&left.1.actual.terminal))
            .then_with(|| {
                left.1
                    .actual
                    .duration_ticks
                    .total_cmp(&right.1.actual.duration_ticks)
            })
            .then_with(|| {
                left.1
                    .transition
                    .value_sample
                    .action
                    .option_id
                    .cmp(&right.1.transition.value_sample.action.option_id)
            })
    });
    candidates
        .into_iter()
        .next()
        .map(|(_, favorable, comparison)| ("rolling_vs_non_rolling", favorable, comparison))
        .ok_or_else(|| {
            evidence_message(
                "native corpora contain no outcome-superior comparable rolling/non-rolling pair",
            )
        })
}

fn straight_pair(
    actions: &[HeldOutAction],
) -> Result<(&'static str, &HeldOutAction, &HeldOutAction), GeneralizedTacticEvidenceError> {
    let mut candidates = Vec::new();
    for favorable in actions {
        for comparison in actions {
            let efficiency_gain = favorable.factors.planned_path_efficiency
                - comparison.factors.planned_path_efficiency;
            let turn_reduction =
                comparison.factors.planned_turn_radians - favorable.factors.planned_turn_radians;
            if favorable.action_sha256 != comparison.action_sha256
                && comparable_source(favorable, comparison)
                && favorable.factors.rolling == comparison.factors.rolling
                && favorable.factors.planned_path_length > 0.0
                && comparison.factors.planned_path_length > 0.0
                && (efficiency_gain >= 0.002 || turn_reduction >= 0.05)
                && favorable.actual.path_efficiency >= comparison.actual.path_efficiency + 0.05
                && favorable.actual.speed_retention + 0.05 >= comparison.actual.speed_retention
                && compare_generalized_tactic_outcomes(&favorable.actual, &comparison.actual)
                    == Ordering::Greater
            {
                candidates.push((
                    (favorable.actual.path_efficiency - comparison.actual.path_efficiency)
                        + (favorable.actual.speed_retention - comparison.actual.speed_retention)
                            .max(0.0),
                    favorable,
                    comparison,
                ));
            }
        }
    }
    candidates.sort_by(|left, right| {
        right.0.total_cmp(&left.0).then_with(|| {
            left.1
                .transition
                .value_sample
                .action
                .option_id
                .cmp(&right.1.transition.value_sample.action.option_id)
        })
    });
    candidates
        .into_iter()
        .next()
        .map(|(_, favorable, comparison)| ("straight_vs_needless_turning", favorable, comparison))
        .ok_or_else(|| {
            evidence_message("native corpora contain no productive straight/turning pair")
        })
}

fn collision_pair(
    actions: &[HeldOutAction],
) -> Result<(&'static str, &HeldOutAction, &HeldOutAction), GeneralizedTacticEvidenceError> {
    let mut candidates = Vec::new();
    for favorable in actions {
        for comparison in actions {
            if favorable.action_sha256 != comparison.action_sha256
                && comparable_source(favorable, comparison)
                && favorable.factors.rolling == comparison.factors.rolling
                && favorable.actual.wall_contact_fraction <= 0.02
                && comparison.actual.wall_contact_fraction
                    >= favorable.actual.wall_contact_fraction + 0.15
                && comparison.actual.momentum_loss_per_tick
                    > favorable.actual.momentum_loss_per_tick
                && compare_generalized_tactic_outcomes(&favorable.actual, &comparison.actual)
                    == Ordering::Greater
            {
                candidates.push((
                    factor_distance(favorable, comparison),
                    favorable,
                    comparison,
                ));
            }
        }
    }
    candidates.sort_by(|left, right| {
        left.0.total_cmp(&right.0).then_with(|| {
            left.1
                .transition
                .value_sample
                .action
                .option_id
                .cmp(&right.1.transition.value_sample.action.option_id)
        })
    });
    candidates
        .into_iter()
        .next()
        .map(|(_, favorable, comparison)| {
            ("clean_motion_vs_wall_momentum_loss", favorable, comparison)
        })
        .ok_or_else(|| evidence_message("native corpora contain no clean/wall-impact pair"))
}

fn evaluate_pair(
    category: &str,
    favorable: &HeldOutAction,
    comparison: &HeldOutAction,
    transitions: &[OptionTransitionSample],
    goal_distance_feature: usize,
) -> Result<GeneralizedTacticHeldOutComparison, GeneralizedTacticEvidenceError> {
    let excluded = BTreeSet::from([favorable.action_sha256, comparison.action_sha256]);
    let training = transitions
        .iter()
        .filter_map(|transition| {
            let digest = transition.value_sample.action.content_sha256().ok()?;
            (!excluded.contains(&digest)).then_some(transition.clone())
        })
        .collect::<Vec<_>>();
    let model = GeneralizedTacticValueModel::fit_transitions(&training, goal_distance_feature)
        .map_err(|error| evidence_message(error.to_string()))?;
    let descriptors = [
        favorable.transition.value_sample.action.clone(),
        comparison.transition.value_sample.action.clone(),
    ];
    let ranked = model
        .rank(
            &favorable.transition.value_sample.state,
            &favorable.context,
            &descriptors,
        )
        .map_err(|error| evidence_message(error.to_string()))?;
    let predicted = |digest: Digest| {
        ranked.iter().find_map(|estimate| {
            estimate
                .descriptor
                .content_sha256()
                .ok()
                .filter(|actual| *actual == digest)
                .map(|_| estimate.outcome)
        })
    };
    let favorable_prediction = predicted(favorable.action_sha256)
        .ok_or_else(|| evidence_message("favorable held-out prediction is absent"))?;
    let comparison_prediction = predicted(comparison.action_sha256)
        .ok_or_else(|| evidence_message("comparison held-out prediction is absent"))?;
    let predicted_first_option_id = ranked
        .first()
        .ok_or_else(|| evidence_message("held-out ranking is empty"))?
        .descriptor
        .option_id
        .clone();
    let passed = ranked
        .first()
        .and_then(|estimate| estimate.descriptor.content_sha256().ok())
        == Some(favorable.action_sha256);
    Ok(GeneralizedTacticHeldOutComparison {
        category: category.into(),
        favorable: report_action(favorable, favorable_prediction),
        comparison: report_action(comparison, comparison_prediction),
        training_transitions_after_exclusion: training.len() as u64,
        predicted_first_option_id,
        passed,
    })
}

fn report_action(
    action: &HeldOutAction,
    predicted: GeneralizedTacticOutcome,
) -> GeneralizedTacticHeldOutAction {
    GeneralizedTacticHeldOutAction {
        option_id: action.transition.value_sample.action.option_id.clone(),
        action_sha256: action.action_sha256,
        before_state_sha256: action.transition.before_state_sha256,
        factors: action.factors,
        actual: action.actual,
        predicted,
    }
}

fn comparable_source(left: &HeldOutAction, right: &HeldOutAction) -> bool {
    left.transition.before_state_sha256 == right.transition.before_state_sha256
        && left.transition.value_sample.state == right.transition.value_sample.state
}

fn factor_distance(left: &HeldOutAction, right: &HeldOutAction) -> f32 {
    let relative =
        |left: f32, right: f32| (left - right).abs() / left.abs().max(right.abs()).max(1.0);
    relative(
        left.factors.planned_path_length,
        right.factors.planned_path_length,
    ) + relative(
        left.factors.planned_displacement,
        right.factors.planned_displacement,
    ) + (left.factors.planned_path_efficiency - right.factors.planned_path_efficiency).abs()
        + (left.factors.planned_turn_radians - right.factors.planned_turn_radians).abs()
        + (left.factors.stick_magnitude - right.factors.stick_magnitude).abs()
}

fn action_geometry_sha256(
    action: &OptionActionDescriptor,
) -> Result<Digest, GeneralizedTacticEvidenceError> {
    let mut geometry = action.clone();
    geometry.option_id.clear();
    geometry.parameters.retain(|name, _| {
        !name.contains("button")
            && name != "controller_sha256"
            && name != "controller_program_sha256"
    });
    let bytes =
        serde_cbor::to_vec(&geometry).map_err(|error| evidence_message(error.to_string()))?;
    Ok(Digest(Sha256::digest(bytes).into()))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GeneralizedTacticEvidenceError(String);

impl fmt::Display for GeneralizedTacticEvidenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for GeneralizedTacticEvidenceError {}

fn evidence_message(message: impl Into<String>) -> GeneralizedTacticEvidenceError {
    GeneralizedTacticEvidenceError(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dusklight_control::option_execution::{OptionParameter, OptionType};

    fn controller(option_id: &str, button_mask: Option<u64>) -> OptionActionDescriptor {
        let mut parameters = BTreeMap::from([
            (
                "command_target_first_x".into(),
                OptionParameter::F32Bits(10.0_f32.to_bits()),
            ),
            (
                "command_target_last_x".into(),
                OptionParameter::F32Bits(100.0_f32.to_bits()),
            ),
            (
                "controller_program_sha256".into(),
                OptionParameter::Text("controller-identity".into()),
            ),
        ]);
        if let Some(button_mask) = button_mask {
            parameters.insert(
                "command_button_mask".into(),
                OptionParameter::Unsigned(button_mask),
            );
            parameters.insert(
                "button_pulse_period_ticks".into(),
                OptionParameter::Unsigned(23),
            );
        }
        OptionActionDescriptor {
            option_id: option_id.into(),
            option_type: OptionType::Custom("reactive_controller".into()),
            parameters,
        }
    }

    #[test]
    fn geometry_identity_ignores_evidence_and_button_schedule_identity() {
        let plain = controller("plain", None);
        let rolling = controller("rolling", Some(0x0100));

        assert_ne!(
            plain.content_sha256().unwrap(),
            rolling.content_sha256().unwrap()
        );
        assert_eq!(
            action_geometry_sha256(&plain).unwrap(),
            action_geometry_sha256(&rolling).unwrap()
        );
    }
}
