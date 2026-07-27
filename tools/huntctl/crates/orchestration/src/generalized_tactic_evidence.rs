//! Native held-out evidence for shared tactic value prediction.
//!
//! The proof never names a desirable action. It derives terminal reachability,
//! first-hit cost, and objective return from authenticated native transition
//! graphs, then checks predictions on unseen episode groups. Exact controller
//! descriptors and a contiguous state region receive independent holdouts.
//! Shuffled-return and objective-blind controls prevent auxiliary measurements
//! or an authored action ordering from satisfying the gate.

use crate::tactic_q_campaign::TacticQTrainingCorpus;
use dusklight_learning::artifact::Digest;
use dusklight_learning::generalized_tactic_value::{
    GeneralizedTacticContext, GeneralizedTacticOutcome, GeneralizedTacticTrainingSample,
    GeneralizedTacticValueModel,
};
use dusklight_learning::option_transition::OptionTransitionSample;
use dusklight_learning::tactic_features::GoalConditionedTacticFeatureEncoder;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

pub const GENERALIZED_TACTIC_HELD_OUT_EVIDENCE_SCHEMA_V2: &str =
    "dusklight-generalized-tactic-held-out-evidence/v2";

const TERMINAL_REWARD: f32 = 100.0;
const NATIVE_TICK_COST: f32 = 0.01;
const MINIMUM_EVALUATION_ROWS: usize = 8;
const MAXIMUM_BACKUP_ITERATIONS: usize = 512;
const METRIC_EPSILON: f32 = 1.0e-6;

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
    pub episode_groups: u64,
    pub objective: GeneralizedTacticEvidenceObjective,
    pub ablated_feature_names: Vec<String>,
    pub holdouts: Vec<GeneralizedTacticHoldoutEvidence>,
    pub passed: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GeneralizedTacticEvidenceObjective {
    pub terminal_reward: f32,
    pub native_tick_cost: f32,
    pub source: String,
    pub auxiliary_signals_affect_policy_utility: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GeneralizedTacticHoldoutEvidence {
    pub kind: String,
    pub training_episode_groups: u64,
    pub evaluation_episode_groups: u64,
    pub training_transitions: u64,
    pub evaluation_transitions: u64,
    pub exact_controller_instances_excluded: u64,
    pub state_region: Option<GeneralizedTacticStateRegion>,
    pub authentic: GeneralizedTacticPredictionMetrics,
    pub typed_signal_ablation: GeneralizedTacticPredictionMetrics,
    pub shuffled_return_control: GeneralizedTacticPredictionMetrics,
    pub auxiliary_only_control: GeneralizedTacticPredictionMetrics,
    pub authentic_beats_shuffled_return: bool,
    pub authentic_beats_auxiliary_only: bool,
    pub typed_signals_improve_prediction: bool,
    pub passed: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GeneralizedTacticStateRegion {
    pub feature_index: usize,
    pub lower_inclusive: f32,
    pub upper_inclusive: f32,
    pub ordering: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GeneralizedTacticPredictionMetrics {
    pub samples: u64,
    pub terminal_positive: u64,
    pub terminal_negative: u64,
    pub terminal_brier_score: f32,
    pub first_hit_tick_mae: Option<f32>,
    pub objective_return_mae: f32,
    pub objective_pair_accuracy: Option<f32>,
    pub mean_nearest_neighbor_distance: f32,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct EpisodeKey {
    corpus: usize,
    group: u64,
}

#[derive(Clone)]
struct NativeRow {
    episode: EpisodeKey,
    action_sha256: Digest,
    transition: OptionTransitionSample,
}

#[derive(Clone, Copy, Debug)]
struct ObjectiveTarget {
    terminal_probability: f32,
    first_hit_ticks: Option<f32>,
    objective_return: f32,
}

#[derive(Clone)]
struct LabeledRow {
    row: NativeRow,
    target: ObjectiveTarget,
}

#[derive(Clone, Copy)]
enum LabelTreatment {
    Authentic,
    Shuffled,
    AuxiliaryOnly,
}

pub fn prove_generalized_tactic_held_out_value(
    corpora: &[TacticQTrainingCorpus],
    goal_distance_feature: usize,
) -> Result<GeneralizedTacticHeldOutEvidence, GeneralizedTacticEvidenceError> {
    let first = corpora
        .first()
        .ok_or_else(|| evidence_message("held-out evidence requires native training corpora"))?;
    if corpora.len() < 5 {
        return Err(evidence_message(
            "held-out evidence requires at least five independent native corpora",
        ));
    }
    if corpora.iter().any(|corpus| {
        corpus.feature_schema_sha256 != first.feature_schema_sha256
            || corpus.objective_sha256 != first.objective_sha256
            || corpus.root_checkpoint_sha256 != first.root_checkpoint_sha256
    }) {
        return Err(evidence_message(
            "held-out corpora do not share feature, objective, and root checkpoint identities",
        ));
    }

    let encoder = GoalConditionedTacticFeatureEncoder::new([0.0; 3])
        .map_err(|error| evidence_message(error.to_string()))?;
    if encoder.schema_sha256 != first.feature_schema_sha256
        || goal_distance_feature != encoder.goal_distance_feature()
    {
        return Err(evidence_message(
            "held-out corpus feature identity or goal-distance index is unsupported",
        ));
    }
    let ablated_indices = typed_signal_ablation_indices(&encoder.feature_names);
    let ablated_feature_names = ablated_indices
        .iter()
        .map(|index| encoder.feature_names[*index].clone())
        .collect::<Vec<_>>();
    if ablated_indices.is_empty() {
        return Err(evidence_message(
            "typed-signal ablation selected no features",
        ));
    }

    let authenticated_native_transitions = corpora
        .iter()
        .map(|corpus| corpus.transitions.len() as u64)
        .sum();
    let rows = flatten_unique_native_rows(corpora)?;
    if rows.len() < MINIMUM_EVALUATION_ROWS * 3 {
        return Err(evidence_message(
            "held-out evidence requires at least 24 unique native transitions",
        ));
    }
    if rows
        .iter()
        .any(|row| row.transition.value_sample.state.len() != encoder.feature_width())
    {
        return Err(evidence_message(
            "held-out transition feature width does not match the authenticated schema",
        ));
    }

    let all_controllers = rows
        .iter()
        .map(|row| row.action_sha256)
        .collect::<BTreeSet<_>>();
    let all_episodes = rows.iter().map(|row| row.episode).collect::<BTreeSet<_>>();
    // Label complete native graphs before applying either holdout. Splitting
    // graph edges first severs successful continuations and silently relabels
    // their prefixes as censored, which makes a controller or state-region
    // generalization test meaningless.
    let labeled = objective_targets(rows)?;

    let controller_holdout = controller_holdout(&labeled, goal_distance_feature, &ablated_indices)?;
    let state_holdout =
        contiguous_state_holdout(&labeled, goal_distance_feature, &ablated_indices)?;
    let holdouts = vec![controller_holdout, state_holdout];
    let passed = holdouts.iter().all(|holdout| holdout.passed);

    Ok(GeneralizedTacticHeldOutEvidence {
        schema: GENERALIZED_TACTIC_HELD_OUT_EVIDENCE_SCHEMA_V2.into(),
        feature_schema_sha256: first.feature_schema_sha256,
        objective_sha256: first.objective_sha256,
        root_checkpoint_sha256: first.root_checkpoint_sha256,
        goal_distance_feature,
        input_corpora: corpora.len() as u64,
        authenticated_native_transitions,
        unique_native_transitions: labeled.len() as u64,
        unique_controller_instances: all_controllers.len() as u64,
        episode_groups: all_episodes.len() as u64,
        objective: GeneralizedTacticEvidenceObjective {
            terminal_reward: TERMINAL_REWARD,
            native_tick_cost: NATIVE_TICK_COST,
            source: "authenticated_terminal_minus_native_input_ticks".into(),
            auxiliary_signals_affect_policy_utility: false,
        },
        ablated_feature_names,
        holdouts,
        passed,
    })
}

fn flatten_unique_native_rows(
    corpora: &[TacticQTrainingCorpus],
) -> Result<Vec<NativeRow>, GeneralizedTacticEvidenceError> {
    let mut unique = BTreeMap::<Digest, NativeRow>::new();
    for (corpus_index, corpus) in corpora.iter().enumerate() {
        if corpus.transitions.len() != corpus.routes.len()
            || corpus.transitions.len() != corpus.episode_groups.len()
        {
            return Err(evidence_message("held-out corpus shape is detached"));
        }
        for (transition, episode_group) in corpus.transitions.iter().zip(&corpus.episode_groups) {
            transition
                .validate()
                .map_err(|error| evidence_message(error.to_string()))?;
            let replay_sha256 = transition
                .replay_identity_sha256()
                .map_err(|error| evidence_message(error.to_string()))?;
            let action_sha256 = transition
                .value_sample
                .action
                .content_sha256()
                .map_err(|error| evidence_message(error.to_string()))?;
            unique.entry(replay_sha256).or_insert_with(|| NativeRow {
                episode: EpisodeKey {
                    corpus: corpus_index,
                    group: *episode_group,
                },
                action_sha256,
                transition: transition.clone(),
            });
        }
    }
    Ok(unique.into_values().collect())
}

fn controller_holdout(
    rows: &[LabeledRow],
    goal_distance_feature: usize,
    ablated_indices: &[usize],
) -> Result<GeneralizedTacticHoldoutEvidence, GeneralizedTacticEvidenceError> {
    let mut controller_ids = rows
        .iter()
        .map(|row| row.row.action_sha256)
        .collect::<Vec<_>>();
    controller_ids.sort_unstable();
    controller_ids.dedup();
    if controller_ids.len() < 2 {
        return Err(evidence_message(
            "exact-controller holdout requires at least two controller instances",
        ));
    }
    // Try each deterministic fifth as the excluded controller fold. Selecting
    // by controller identity (rather than corpus) keeps entire native graphs
    // labeled while still guaranteeing that no exact held-out controller is
    // visible to the fit.
    for fold in 0..5 {
        let held_out_controllers = controller_ids
            .iter()
            .enumerate()
            .filter_map(|(index, digest)| (index % 5 == fold).then_some(*digest))
            .collect::<BTreeSet<_>>();
        let (training, evaluation): (Vec<_>, Vec<_>) = rows
            .iter()
            .cloned()
            .partition(|row| !held_out_controllers.contains(&row.row.action_sha256));
        if !valid_mixed_partition(&training, &evaluation) {
            continue;
        }
        return evaluate_holdout(
            "exact_controller_instances",
            training,
            evaluation,
            held_out_controllers.len(),
            None,
            goal_distance_feature,
            ablated_indices,
        );
    }
    Err(evidence_message(
        "exact-controller folds do not contain enough mixed terminal and censored evidence",
    ))
}

fn contiguous_state_holdout(
    rows: &[LabeledRow],
    goal_distance_feature: usize,
    ablated_indices: &[usize],
) -> Result<GeneralizedTacticHoldoutEvidence, GeneralizedTacticEvidenceError> {
    let mut distances = rows
        .iter()
        .map(|row| row.row.transition.value_sample.state[goal_distance_feature])
        .collect::<Vec<_>>();
    distances.sort_by(f32::total_cmp);
    distances.dedup_by(|left, right| left.to_bits() == right.to_bits());
    if distances.len() < 5 {
        return Err(evidence_message(
            "native episodes do not span enough distinct states for a contiguous-region holdout",
        ));
    }
    let lower = distances[distances.len() * 2 / 5];
    let upper = distances[distances.len() * 3 / 5];
    if lower >= upper {
        return Err(evidence_message(
            "contiguous state-region holdout has zero width",
        ));
    }
    let outside = |row: &&LabeledRow| {
        let distance = row.row.transition.value_sample.state[goal_distance_feature];
        distance < lower || distance > upper
    };
    let inside = |row: &&LabeledRow| {
        let distance = row.row.transition.value_sample.state[goal_distance_feature];
        distance >= lower && distance <= upper
    };
    let training = rows.iter().filter(outside).cloned().collect::<Vec<_>>();
    let evaluation = rows.iter().filter(inside).cloned().collect::<Vec<_>>();
    evaluate_holdout(
        "contiguous_goal_distance_region",
        training,
        evaluation,
        0,
        Some(GeneralizedTacticStateRegion {
            feature_index: goal_distance_feature,
            lower_inclusive: lower,
            upper_inclusive: upper,
            ordering: "ascending_goal_planar_distance_middle_quintile".into(),
        }),
        goal_distance_feature,
        ablated_indices,
    )
}

#[allow(clippy::too_many_arguments)]
fn evaluate_holdout(
    kind: &str,
    training: Vec<LabeledRow>,
    evaluation: Vec<LabeledRow>,
    exact_controller_instances_excluded: usize,
    state_region: Option<GeneralizedTacticStateRegion>,
    goal_distance_feature: usize,
    ablated_indices: &[usize],
) -> Result<GeneralizedTacticHoldoutEvidence, GeneralizedTacticEvidenceError> {
    if training.len() < MINIMUM_EVALUATION_ROWS || evaluation.len() < MINIMUM_EVALUATION_ROWS {
        return Err(evidence_message(format!(
            "{kind} holdout requires at least {MINIMUM_EVALUATION_ROWS} training and evaluation transitions"
        )));
    }
    if !has_mixed_terminal_outcomes(&training) {
        return Err(evidence_message(format!(
            "{kind} holdout training requires both terminal-reaching and censored native outcomes"
        )));
    }
    let positives = evaluation
        .iter()
        .filter(|row| row.target.terminal_probability == 1.0)
        .count();
    if positives == 0 || positives == evaluation.len() {
        return Err(evidence_message(format!(
            "{kind} holdout requires both terminal-reaching and censored native outcomes"
        )));
    }

    let authentic_model = fit_evidence_model(
        &training,
        goal_distance_feature,
        &[],
        LabelTreatment::Authentic,
    )?;
    let ablated_model = fit_evidence_model(
        &training,
        goal_distance_feature,
        ablated_indices,
        LabelTreatment::Authentic,
    )?;
    let shuffled_model = fit_evidence_model(
        &training,
        goal_distance_feature,
        &[],
        LabelTreatment::Shuffled,
    )?;
    let auxiliary_only_model = fit_evidence_model(
        &training,
        goal_distance_feature,
        &[],
        LabelTreatment::AuxiliaryOnly,
    )?;

    let authentic = prediction_metrics(&authentic_model, &evaluation, &[])?;
    let typed_signal_ablation = prediction_metrics(&ablated_model, &evaluation, ablated_indices)?;
    let shuffled_return_control = prediction_metrics(&shuffled_model, &evaluation, &[])?;
    let auxiliary_only_control = prediction_metrics(&auxiliary_only_model, &evaluation, &[])?;

    let authentic_beats_shuffled_return = better_than_control(&authentic, &shuffled_return_control);
    let authentic_beats_auxiliary_only = better_than_control(&authentic, &auxiliary_only_control);
    let typed_signals_improve_prediction =
        improves_any_prediction(&authentic, &typed_signal_ablation);
    let passed = authentic_beats_shuffled_return
        && authentic_beats_auxiliary_only
        && typed_signals_improve_prediction;

    Ok(GeneralizedTacticHoldoutEvidence {
        kind: kind.into(),
        training_episode_groups: training
            .iter()
            .map(|row| row.row.episode)
            .collect::<BTreeSet<_>>()
            .len() as u64,
        evaluation_episode_groups: evaluation
            .iter()
            .map(|row| row.row.episode)
            .collect::<BTreeSet<_>>()
            .len() as u64,
        training_transitions: training.len() as u64,
        evaluation_transitions: evaluation.len() as u64,
        exact_controller_instances_excluded: exact_controller_instances_excluded as u64,
        state_region,
        authentic,
        typed_signal_ablation,
        shuffled_return_control,
        auxiliary_only_control,
        authentic_beats_shuffled_return,
        authentic_beats_auxiliary_only,
        typed_signals_improve_prediction,
        passed,
    })
}

fn valid_mixed_partition(training: &[LabeledRow], evaluation: &[LabeledRow]) -> bool {
    training.len() >= MINIMUM_EVALUATION_ROWS
        && evaluation.len() >= MINIMUM_EVALUATION_ROWS
        && has_mixed_terminal_outcomes(training)
        && has_mixed_terminal_outcomes(evaluation)
}

fn has_mixed_terminal_outcomes(rows: &[LabeledRow]) -> bool {
    let positives = rows
        .iter()
        .filter(|row| row.target.terminal_probability == 1.0)
        .count();
    positives > 0 && positives < rows.len()
}

fn objective_targets(
    rows: Vec<NativeRow>,
) -> Result<Vec<LabeledRow>, GeneralizedTacticEvidenceError> {
    let mut outgoing = BTreeMap::<(EpisodeKey, Digest), Vec<usize>>::new();
    for (index, row) in rows.iter().enumerate() {
        outgoing
            .entry((row.episode, row.transition.before_state_sha256))
            .or_default()
            .push(index);
    }
    let immediate = rows
        .iter()
        .map(|row| {
            let ticks = row.transition.value_sample.duration_ticks as f32;
            -NATIVE_TICK_COST * ticks
                + if row.transition.value_sample.terminal {
                    TERMINAL_REWARD
                } else {
                    0.0
                }
        })
        .collect::<Vec<_>>();
    let mut objective = immediate.clone();
    let mut terminal = rows
        .iter()
        .map(|row| f32::from(row.transition.value_sample.terminal))
        .collect::<Vec<_>>();
    let mut first_hit_ticks = rows
        .iter()
        .map(|row| {
            row.transition
                .value_sample
                .terminal
                .then_some(row.transition.value_sample.duration_ticks as f32)
        })
        .collect::<Vec<_>>();

    for _ in 0..rows.len().min(MAXIMUM_BACKUP_ITERATIONS).max(1) {
        let prior_objective = objective.clone();
        let prior_terminal = terminal.clone();
        let prior_ticks = first_hit_ticks.clone();
        let mut changed = false;
        for (index, row) in rows.iter().enumerate() {
            if row.transition.value_sample.terminal {
                continue;
            }
            let successor = outgoing
                .get(&(row.episode, row.transition.after_state_sha256))
                .and_then(|indices| {
                    indices.iter().copied().max_by(|left, right| {
                        prior_objective[*left].total_cmp(&prior_objective[*right])
                    })
                });
            let (next_objective, next_terminal, next_ticks) = successor
                .map(|next| {
                    (
                        prior_objective[next],
                        prior_terminal[next],
                        prior_ticks[next],
                    )
                })
                .unwrap_or((0.0, 0.0, None));
            let value = immediate[index] + next_objective;
            let ticks =
                next_ticks.map(|ticks| ticks + row.transition.value_sample.duration_ticks as f32);
            changed |= (objective[index] - value).abs() > METRIC_EPSILON
                || (terminal[index] - next_terminal).abs() > METRIC_EPSILON
                || first_hit_ticks[index] != ticks;
            objective[index] = value;
            terminal[index] = next_terminal;
            first_hit_ticks[index] = ticks;
        }
        if !changed {
            break;
        }
    }

    if objective.iter().any(|value| !value.is_finite()) {
        return Err(evidence_message(
            "native objective backup produced a non-finite return",
        ));
    }
    Ok(rows
        .into_iter()
        .zip(objective)
        .zip(terminal)
        .zip(first_hit_ticks)
        .map(
            |(((row, objective_return), terminal_probability), first_hit_ticks)| LabeledRow {
                row,
                target: ObjectiveTarget {
                    terminal_probability,
                    first_hit_ticks,
                    objective_return,
                },
            },
        )
        .collect())
}

fn fit_evidence_model(
    rows: &[LabeledRow],
    goal_distance_feature: usize,
    ablated_indices: &[usize],
    treatment: LabelTreatment,
) -> Result<GeneralizedTacticValueModel, GeneralizedTacticEvidenceError> {
    let mut targets = rows.iter().map(|row| row.target).collect::<Vec<_>>();
    match treatment {
        LabelTreatment::Authentic => {}
        LabelTreatment::Shuffled => {
            let rotation = (targets.len() / 2).max(1);
            targets.rotate_left(rotation);
        }
        LabelTreatment::AuxiliaryOnly => {
            let count = targets.len() as f32;
            let mean_terminal = targets
                .iter()
                .map(|target| target.terminal_probability)
                .sum::<f32>()
                / count;
            let mean_return = targets
                .iter()
                .map(|target| target.objective_return)
                .sum::<f32>()
                / count;
            let positive_ticks = targets
                .iter()
                .filter_map(|target| target.first_hit_ticks)
                .collect::<Vec<_>>();
            let mean_ticks = (!positive_ticks.is_empty())
                .then(|| positive_ticks.iter().sum::<f32>() / positive_ticks.len() as f32);
            targets.fill(ObjectiveTarget {
                terminal_probability: mean_terminal,
                first_hit_ticks: mean_ticks,
                objective_return: mean_return,
            });
        }
    }
    let samples = rows
        .iter()
        .zip(targets)
        .map(|(row, target)| {
            let mut state_features = row.row.transition.value_sample.state.clone();
            ablate_state(&mut state_features, ablated_indices);
            let mut context = GeneralizedTacticContext::from_facts(&row.row.transition.before)
                .map_err(|error| evidence_message(error.to_string()))?;
            ablate_context(&mut context, ablated_indices);
            let mut outcome = GeneralizedTacticOutcome::from_transition(
                &row.row.transition,
                goal_distance_feature,
            )
            .map_err(|error| evidence_message(error.to_string()))?;
            outcome.terminal = target.terminal_probability;
            outcome.reward = target.objective_return;
            outcome.duration_ticks = target.first_hit_ticks.unwrap_or(0.0);
            Ok(GeneralizedTacticTrainingSample {
                state_features,
                context,
                action: row.row.transition.value_sample.action.clone(),
                outcome,
            })
        })
        .collect::<Result<Vec<_>, GeneralizedTacticEvidenceError>>()?;
    GeneralizedTacticValueModel::fit(&samples).map_err(|error| evidence_message(error.to_string()))
}

fn prediction_metrics(
    model: &GeneralizedTacticValueModel,
    rows: &[LabeledRow],
    ablated_indices: &[usize],
) -> Result<GeneralizedTacticPredictionMetrics, GeneralizedTacticEvidenceError> {
    let mut predictions = Vec::with_capacity(rows.len());
    let mut brier = 0.0_f32;
    let mut first_hit_error = 0.0_f32;
    let mut first_hit_count = 0_u64;
    let mut return_error = 0.0_f32;
    let mut neighbor_distance = 0.0_f32;
    for row in rows {
        let mut state = row.row.transition.value_sample.state.clone();
        ablate_state(&mut state, ablated_indices);
        let mut context = GeneralizedTacticContext::from_facts(&row.row.transition.before)
            .map_err(|error| evidence_message(error.to_string()))?;
        ablate_context(&mut context, ablated_indices);
        let prediction = model
            .predict(&state, &context, &row.row.transition.value_sample.action)
            .map_err(|error| evidence_message(error.to_string()))?;
        let predicted_terminal = prediction.outcome.terminal.clamp(0.0, 1.0);
        brier += (predicted_terminal - row.target.terminal_probability).powi(2);
        return_error += (prediction.outcome.reward - row.target.objective_return).abs();
        if let Some(actual_ticks) = row.target.first_hit_ticks {
            first_hit_error += (prediction.outcome.duration_ticks - actual_ticks).abs();
            first_hit_count += 1;
        }
        neighbor_distance += prediction.nearest_distance;
        predictions.push(prediction.outcome.reward);
    }
    let count = rows.len() as f32;
    let terminal_positive = rows
        .iter()
        .filter(|row| row.target.terminal_probability == 1.0)
        .count() as u64;
    Ok(GeneralizedTacticPredictionMetrics {
        samples: rows.len() as u64,
        terminal_positive,
        terminal_negative: rows.len() as u64 - terminal_positive,
        terminal_brier_score: brier / count,
        first_hit_tick_mae: (first_hit_count > 0)
            .then_some(first_hit_error / first_hit_count as f32),
        objective_return_mae: return_error / count,
        objective_pair_accuracy: objective_pair_accuracy(rows, &predictions),
        mean_nearest_neighbor_distance: neighbor_distance / count,
    })
}

fn objective_pair_accuracy(rows: &[LabeledRow], predictions: &[f32]) -> Option<f32> {
    let mut correct = 0.0_f32;
    let mut compared = 0_u64;
    'outer: for left in 0..rows.len() {
        for right in (left + 1)..rows.len() {
            let actual = rows[left]
                .target
                .objective_return
                .total_cmp(&rows[right].target.objective_return);
            if actual == std::cmp::Ordering::Equal {
                continue;
            }
            let predicted = predictions[left].total_cmp(&predictions[right]);
            correct += if actual == predicted {
                1.0
            } else if predicted == std::cmp::Ordering::Equal {
                0.5
            } else {
                0.0
            };
            compared += 1;
            if compared == 50_000 {
                break 'outer;
            }
        }
    }
    (compared > 0).then_some(correct / compared as f32)
}

fn better_than_control(
    authentic: &GeneralizedTacticPredictionMetrics,
    control: &GeneralizedTacticPredictionMetrics,
) -> bool {
    let lower_error = authentic.objective_return_mae + METRIC_EPSILON
        < control.objective_return_mae
        && authentic.terminal_brier_score + METRIC_EPSILON < control.terminal_brier_score;
    let pair_better = match (
        authentic.objective_pair_accuracy,
        control.objective_pair_accuracy,
    ) {
        (Some(authentic), Some(control)) => authentic > control + METRIC_EPSILON,
        _ => false,
    };
    lower_error && pair_better
}

fn improves_any_prediction(
    authentic: &GeneralizedTacticPredictionMetrics,
    ablated: &GeneralizedTacticPredictionMetrics,
) -> bool {
    authentic.objective_return_mae + METRIC_EPSILON < ablated.objective_return_mae
        || authentic.terminal_brier_score + METRIC_EPSILON < ablated.terminal_brier_score
        || matches!(
            (authentic.first_hit_tick_mae, ablated.first_hit_tick_mae),
            (Some(full), Some(reduced)) if full + METRIC_EPSILON < reduced
        )
}

fn typed_signal_ablation_indices(feature_names: &[String]) -> Vec<usize> {
    feature_names
        .iter()
        .enumerate()
        .filter_map(|(index, name)| {
            (name.starts_with("history_")
                || name.starts_with("trajectory_")
                || name.starts_with("recent_option_")
                || name.starts_with("velocity_")
                || name.starts_with("forward_speed")
                || name.starts_with("player_contacts")
                || name.starts_with("collision_correction")
                || name.starts_with("player_procedure")
                || name.starts_with("player_mode")
                || name.starts_with("yaw_")
                || name.starts_with("camera_yaw_")
                || name.starts_with("goal_history_")
                || name.starts_with("goal_trajectory_")
                || name == "goal_closing_speed")
                .then_some(index)
        })
        .collect()
}

fn ablate_state(state: &mut [f32], indices: &[usize]) {
    for index in indices {
        state[*index] = 0.0;
    }
}

fn ablate_context(context: &mut GeneralizedTacticContext, indices: &[usize]) {
    if indices.is_empty() {
        return;
    }
    context.velocity_x = 0.0;
    context.velocity_z = 0.0;
    context.forward_speed = 0.0;
    context.yaw_sin = 0.0;
    context.yaw_cos = 0.0;
    context.camera_yaw_sin = 0.0;
    context.camera_yaw_cos = 0.0;
    context.contacts = 0.0;
    context.collision_correction = 0.0;
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

    #[test]
    fn typed_signal_ablation_keeps_position_and_goal_distance() {
        let encoder = GoalConditionedTacticFeatureEncoder::new([1.0, 2.0, 3.0]).unwrap();
        let indices = typed_signal_ablation_indices(&encoder.feature_names);
        let names = indices
            .iter()
            .map(|index| encoder.feature_names[*index].as_str())
            .collect::<BTreeSet<_>>();

        assert!(names.contains("trajectory_straightness"));
        assert!(names.contains("recent_option_momentum_loss_per_tick"));
        assert!(names.contains("player_procedure"));
        assert!(names.contains("goal_closing_speed"));
        assert!(!names.contains("player_x"));
        assert!(!names.contains("goal_planar_distance"));
    }

    #[test]
    fn controls_must_improve_errors_and_pair_ordering() {
        let authentic = GeneralizedTacticPredictionMetrics {
            samples: 10,
            terminal_positive: 5,
            terminal_negative: 5,
            terminal_brier_score: 0.1,
            first_hit_tick_mae: Some(2.0),
            objective_return_mae: 3.0,
            objective_pair_accuracy: Some(0.8),
            mean_nearest_neighbor_distance: 0.1,
        };
        let mut control = authentic.clone();
        control.terminal_brier_score = 0.2;
        control.objective_return_mae = 4.0;
        control.objective_pair_accuracy = Some(0.6);
        assert!(better_than_control(&authentic, &control));

        control.objective_pair_accuracy = Some(0.9);
        assert!(!better_than_control(&authentic, &control));
    }

    #[test]
    fn proof_rejects_a_non_goal_feature_as_goal_distance() {
        let encoder = GoalConditionedTacticFeatureEncoder::new([0.0; 3]).unwrap();
        let corpus = TacticQTrainingCorpus {
            feature_schema_sha256: encoder.schema_sha256,
            objective_sha256: Digest([1; 32]),
            root_checkpoint_sha256: Digest([2; 32]),
            transitions: Vec::new(),
            routes: Vec::new(),
            episode_groups: Vec::new(),
        };
        let error = prove_generalized_tactic_held_out_value(
            &vec![corpus; 5],
            encoder.goal_distance_feature() - 1,
        )
        .unwrap_err();

        assert!(error.to_string().contains("goal-distance index"));
    }
}
