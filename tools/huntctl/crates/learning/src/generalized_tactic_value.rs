//! Shared state-action outcome prediction for unseen tactic controllers.
//!
//! The existing option FQI remains authoritative for actions with exact replay
//! support. This model serves acquisition: it represents executable command
//! factors and observed native outcomes so a new controller can inherit
//! evidence from similar actions instead of becoming an unrelated categorical
//! arm. Option IDs and content digests are deliberately excluded from features.

use crate::artifact::Digest;
use crate::fact_snapshot::{FactSnapshot, OptionTrajectoryFactSnapshot};
use crate::option_transition::OptionTransitionSample;
use crate::option_values::OptionActionDescriptor;
use dusklight_control::option_execution::{OptionParameter, OptionType};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

pub const GENERALIZED_TACTIC_ACTION_FEATURE_WIDTH: usize = 71;
const MAX_GENERALIZED_TACTIC_SAMPLES: usize = 100_000;
const MAX_FITTED_Q_BACKUP_ITERATIONS: usize = 512;
const NEIGHBORS: usize = 8;
const STATE_NEIGHBORS: usize = 16;
const EXACT_STATE_DISTANCE_EPSILON: f32 = 1.0e-8;
const RETURN_COMPARISON_RESOLUTION: f64 = 1.0e-4;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GeneralizedTacticContext {
    pub player_x: f32,
    pub player_z: f32,
    pub velocity_x: f32,
    pub velocity_z: f32,
    pub forward_speed: f32,
    pub yaw_sin: f32,
    pub yaw_cos: f32,
    pub camera_yaw_sin: f32,
    pub camera_yaw_cos: f32,
    pub contacts: f32,
    pub collision_correction: f32,
}

impl GeneralizedTacticContext {
    pub fn from_facts(facts: &FactSnapshot) -> Result<Self, GeneralizedTacticValueError> {
        facts
            .validate()
            .map_err(|error| GeneralizedTacticValueError::InvalidFacts(error.to_string()))?;
        let position = facts.player.position_f32_bits.map(f32::from_bits);
        let velocity = facts
            .player
            .velocity_f32_bits
            .map(|value| value.map(f32::from_bits))
            .unwrap_or([0.0; 3]);
        let forward_speed = facts
            .player
            .forward_speed_f32_bits
            .map(f32::from_bits)
            .unwrap_or(0.0);
        let yaw = facts
            .player
            .current_angle
            .map(|angle| f32::from(angle[1]) * std::f32::consts::PI / 32768.0);
        let camera_yaw = facts.player.camera_yaw_radians_f32_bits.map(f32::from_bits);
        let collision_correction = facts
            .player
            .collision_correction_f32_bits
            .map(|value| {
                let value = value.map(f32::from_bits);
                value[0].hypot(value[1])
            })
            .unwrap_or(0.0);
        let context = Self {
            player_x: position[0],
            player_z: position[2],
            velocity_x: velocity[0],
            velocity_z: velocity[2],
            forward_speed,
            yaw_sin: yaw.map(f32::sin).unwrap_or(0.0),
            yaw_cos: yaw.map(f32::cos).unwrap_or(0.0),
            camera_yaw_sin: camera_yaw.map(f32::sin).unwrap_or(0.0),
            camera_yaw_cos: camera_yaw.map(f32::cos).unwrap_or(0.0),
            contacts: f32::from(facts.player.contacts.unwrap_or(0)),
            collision_correction,
        };
        if context.values().iter().any(|value| !value.is_finite()) {
            return Err(GeneralizedTacticValueError::NonFinite);
        }
        Ok(context)
    }

    fn values(self) -> [f32; 11] {
        [
            self.player_x,
            self.player_z,
            self.velocity_x,
            self.velocity_z,
            self.forward_speed,
            self.yaw_sin,
            self.yaw_cos,
            self.camera_yaw_sin,
            self.camera_yaw_cos,
            self.contacts,
            self.collision_correction,
        ]
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct GeneralizedTacticOutcome {
    pub terminal: f32,
    pub reward: f32,
    pub duration_ticks: f32,
    pub goal_progress_per_tick: f32,
    pub path_efficiency: f32,
    pub speed_retention: f32,
    pub stalled_command_fraction: f32,
    pub wall_contact_fraction: f32,
    pub momentum_loss_per_tick: f32,
    pub collision_correction_per_tick: f32,
}

impl GeneralizedTacticOutcome {
    pub fn from_transition(
        transition: &OptionTransitionSample,
        goal_distance_feature: usize,
    ) -> Result<Self, GeneralizedTacticValueError> {
        transition
            .validate()
            .map_err(|error| GeneralizedTacticValueError::InvalidTransition(error.to_string()))?;
        let duration = transition.value_sample.duration_ticks as f32;
        let before_distance = transition
            .value_sample
            .state
            .get(goal_distance_feature)
            .copied()
            .ok_or(GeneralizedTacticValueError::FeatureWidth)?;
        let after_distance = transition
            .value_sample
            .next_state
            .get(goal_distance_feature)
            .copied()
            .ok_or(GeneralizedTacticValueError::FeatureWidth)?;
        let trajectory = transition
            .after
            .recent_option
            .as_ref()
            .filter(|recent| recent.option_id == transition.value_sample.action.option_id)
            .and_then(|recent| recent.trajectory.as_ref());
        let mut outcome = Self {
            terminal: f32::from(transition.value_sample.terminal),
            reward: transition.value_sample.reward,
            duration_ticks: duration,
            goal_progress_per_tick: (before_distance - after_distance) / duration,
            ..Self::default()
        };
        if let Some(trajectory) = trajectory {
            outcome.apply_trajectory(trajectory);
        }
        if outcome.values().iter().any(|value| !value.is_finite()) {
            return Err(GeneralizedTacticValueError::NonFinite);
        }
        Ok(outcome)
    }

    fn apply_trajectory(&mut self, trajectory: &OptionTrajectoryFactSnapshot) {
        let ticks = trajectory.observed_ticks.max(1) as f32;
        let path = f32::from_bits(trajectory.planar_path_length_f32_bits);
        let displacement = f32::from_bits(trajectory.planar_displacement_f32_bits);
        let final_velocity = f32::from_bits(trajectory.final_planar_velocity_f32_bits);
        let maximum_velocity = f32::from_bits(trajectory.maximum_planar_velocity_f32_bits);
        self.path_efficiency = if path > f32::EPSILON {
            (displacement / path).clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.speed_retention = if maximum_velocity > f32::EPSILON {
            (final_velocity / maximum_velocity).clamp(0.0, 4.0)
        } else {
            0.0
        };
        self.stalled_command_fraction = trajectory.commanded_stall_ticks as f32
            / trajectory.commanded_motion_ticks.max(1) as f32;
        self.wall_contact_fraction = trajectory.wall_contact_ticks as f32 / ticks;
        self.momentum_loss_per_tick =
            f32::from_bits(trajectory.commanded_momentum_loss_f32_bits) / ticks;
        self.collision_correction_per_tick =
            f32::from_bits(trajectory.collision_correction_total_f32_bits) / ticks;
    }

    fn values(self) -> [f32; 10] {
        [
            self.terminal,
            self.reward,
            self.duration_ticks,
            self.goal_progress_per_tick,
            self.path_efficiency,
            self.speed_retention,
            self.stalled_command_fraction,
            self.wall_contact_fraction,
            self.momentum_loss_per_tick,
            self.collision_correction_per_tick,
        ]
    }

    fn weighted_add(&mut self, other: Self, weight: f32) {
        self.terminal += other.terminal * weight;
        self.reward += other.reward * weight;
        self.duration_ticks += other.duration_ticks * weight;
        self.goal_progress_per_tick += other.goal_progress_per_tick * weight;
        self.path_efficiency += other.path_efficiency * weight;
        self.speed_retention += other.speed_retention * weight;
        self.stalled_command_fraction += other.stalled_command_fraction * weight;
        self.wall_contact_fraction += other.wall_contact_fraction * weight;
        self.momentum_loss_per_tick += other.momentum_loss_per_tick * weight;
        self.collision_correction_per_tick += other.collision_correction_per_tick * weight;
    }

    fn scale(&mut self, scale: f32) {
        self.terminal *= scale;
        self.reward *= scale;
        self.duration_ticks *= scale;
        self.goal_progress_per_tick *= scale;
        self.path_efficiency *= scale;
        self.speed_retention *= scale;
        self.stalled_command_fraction *= scale;
        self.wall_contact_fraction *= scale;
        self.momentum_loss_per_tick *= scale;
        self.collision_correction_per_tick *= scale;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct GeneralizedTacticActionFactors {
    pub planned_path_length: f32,
    pub planned_displacement: f32,
    pub planned_path_efficiency: f32,
    pub planned_turn_radians: f32,
    pub stick_magnitude: f32,
    pub waypoint_switch_radius: f32,
    pub button_active_fraction: f32,
    pub button_pulse_count: f32,
    pub button_mean_interval_ticks: f32,
    pub button_phase_fraction: f32,
    pub button_mask: u16,
    pub rolling: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GeneralizedTacticTrainingSample {
    pub state_features: Vec<f32>,
    pub context: GeneralizedTacticContext,
    pub action: OptionActionDescriptor,
    pub outcome: GeneralizedTacticOutcome,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GeneralizedTacticEstimate {
    pub descriptor: OptionActionDescriptor,
    pub outcome: GeneralizedTacticOutcome,
    pub nearest_distance: f32,
    pub neighbors: usize,
}

#[derive(Clone, Debug)]
struct EncodedSample {
    state: Vec<f32>,
    action: [f32; GENERALIZED_TACTIC_ACTION_FEATURE_WIDTH],
    outcome: GeneralizedTacticOutcome,
}

#[derive(Clone, Debug)]
pub struct GeneralizedTacticValueModel {
    state_min: Vec<f32>,
    state_range: Vec<f32>,
    action_min: [f32; GENERALIZED_TACTIC_ACTION_FEATURE_WIDTH],
    action_range: [f32; GENERALIZED_TACTIC_ACTION_FEATURE_WIDTH],
    samples: Vec<EncodedSample>,
}

impl GeneralizedTacticValueModel {
    pub fn fit_transitions(
        transitions: &[OptionTransitionSample],
        goal_distance_feature: usize,
    ) -> Result<Self, GeneralizedTacticValueError> {
        let samples = transitions
            .iter()
            .map(|transition| {
                Ok(GeneralizedTacticTrainingSample {
                    state_features: transition.value_sample.state.clone(),
                    context: GeneralizedTacticContext::from_facts(&transition.before)?,
                    action: transition.value_sample.action.clone(),
                    outcome: GeneralizedTacticOutcome::from_transition(
                        transition,
                        goal_distance_feature,
                    )?,
                })
            })
            .collect::<Result<Vec<_>, GeneralizedTacticValueError>>()?;
        Self::fit(&samples)
    }

    /// Fits a shared semi-Markov action-value model.
    ///
    /// Bellman backups use only the authenticated scalar reward and observed
    /// successor actions. Auxiliary trajectory outcomes are retained for
    /// inspection, but cannot enter the objective return or action ordering.
    /// Nonterminal leaves are right-censored observations, not failed terminal
    /// episodes. They remain in the fit as reachability/support evidence, but
    /// carry only their observed immediate return; Bellman backups never invent
    /// an unobserved continuation for a censored component.
    pub fn fit_fitted_q_transitions(
        transitions: &[OptionTransitionSample],
        goal_distance_feature: usize,
        iterations: usize,
        per_tick_discount: f32,
    ) -> Result<Self, GeneralizedTacticValueError> {
        if iterations == 0
            || iterations > MAX_FITTED_Q_BACKUP_ITERATIONS
            || !per_tick_discount.is_finite()
            || !(0.0..=1.0).contains(&per_tick_discount)
            || per_tick_discount == 0.0
        {
            return Err(GeneralizedTacticValueError::InvalidConfig);
        }
        let mut samples = transitions
            .iter()
            .map(|transition| {
                Ok(GeneralizedTacticTrainingSample {
                    state_features: transition.value_sample.state.clone(),
                    context: GeneralizedTacticContext::from_facts(&transition.before)?,
                    action: transition.value_sample.action.clone(),
                    outcome: GeneralizedTacticOutcome::from_transition(
                        transition,
                        goal_distance_feature,
                    )?,
                })
            })
            .collect::<Result<Vec<_>, GeneralizedTacticValueError>>()?;
        let mut outgoing = BTreeMap::<_, Vec<usize>>::new();
        for (index, transition) in transitions.iter().enumerate() {
            outgoing
                .entry(transition.before_state_sha256)
                .or_default()
                .push(index);
        }
        let terminal_supported = terminal_supported_transition_indices(transitions);
        let immediate = transitions
            .iter()
            .map(|transition| transition.value_sample.reward)
            .collect::<Vec<_>>();
        let mut values = immediate.clone();
        let backup_limit = fitted_q_backup_limit(iterations, transitions.len());
        for iteration in 0..backup_limit {
            let prior = values.clone();
            let mut changed = false;
            for (index, transition) in transitions.iter().enumerate() {
                values[index] = if transition.value_sample.terminal
                    || !terminal_supported.contains(&index)
                {
                    immediate[index]
                } else if let Some(next_actions) = outgoing.get(&transition.after_state_sha256) {
                    let next_value = next_actions
                        .iter()
                        .filter(|next| terminal_supported.contains(next))
                        .map(|next| prior[*next])
                        .max_by(f32::total_cmp)
                        .ok_or(GeneralizedTacticValueError::InvalidTransition(
                            "terminal-supported successor action set is empty".into(),
                        ))?;
                    let duration =
                        i32::try_from(transition.value_sample.duration_ticks).map_err(|_| {
                            GeneralizedTacticValueError::InvalidTransition(
                                "action duration exceeds fitted-Q discount bounds".into(),
                            )
                        })?;
                    immediate[index] + per_tick_discount.powi(duration) * next_value
                } else {
                    immediate[index]
                };
                if !values[index].is_finite() {
                    return Err(GeneralizedTacticValueError::NonFinite);
                }
                changed |= (values[index] - prior[index]).abs() > 1.0e-6;
            }
            if iteration + 1 >= iterations && !changed {
                break;
            }
        }
        let first_hit_ticks =
            terminal_supported_first_hit_ticks(transitions, &terminal_supported, backup_limit)?;
        for (index, (sample, value)) in samples.iter_mut().zip(values).enumerate() {
            sample.outcome.reward = value;
            sample.outcome.terminal = f32::from(terminal_supported.contains(&index));
            sample.outcome.duration_ticks = first_hit_ticks[index].unwrap_or(0) as f32;
        }
        Self::fit(&samples)
    }

    pub fn fit(
        samples: &[GeneralizedTacticTrainingSample],
    ) -> Result<Self, GeneralizedTacticValueError> {
        if samples.len() < 2 || samples.len() > MAX_GENERALIZED_TACTIC_SAMPLES {
            return Err(GeneralizedTacticValueError::SampleCount);
        }
        let state_width = samples[0].state_features.len();
        if state_width == 0
            || samples.iter().any(|sample| {
                sample.state_features.len() != state_width
                    || sample
                        .state_features
                        .iter()
                        .chain(sample.outcome.values().iter())
                        .any(|value| !value.is_finite())
            })
        {
            return Err(GeneralizedTacticValueError::FeatureWidth);
        }
        let encoded = samples
            .iter()
            .map(|sample| {
                Ok(EncodedSample {
                    state: sample.state_features.clone(),
                    action: encode_action(&sample.context, &sample.action)?,
                    outcome: sample.outcome,
                })
            })
            .collect::<Result<Vec<_>, GeneralizedTacticValueError>>()?;
        let (state_min, state_range) = feature_ranges(
            encoded.iter().map(|sample| sample.state.as_slice()),
            state_width,
        );
        let (action_min, action_range) = action_feature_ranges(&encoded);
        Ok(Self {
            state_min,
            state_range,
            action_min,
            action_range,
            samples: encoded,
        })
    }

    pub fn predict(
        &self,
        state_features: &[f32],
        context: &GeneralizedTacticContext,
        descriptor: &OptionActionDescriptor,
    ) -> Result<GeneralizedTacticEstimate, GeneralizedTacticValueError> {
        if state_features.len() != self.state_min.len()
            || state_features.iter().any(|value| !value.is_finite())
        {
            return Err(GeneralizedTacticValueError::FeatureWidth);
        }
        let action = encode_action(context, descriptor)?;
        let mut state_neighbors = self
            .samples
            .iter()
            .map(|sample| {
                (
                    normalized_distance(
                        state_features,
                        &sample.state,
                        &self.state_min,
                        &self.state_range,
                    ),
                    sample,
                )
            })
            .collect::<Vec<_>>();
        state_neighbors.sort_by(|left, right| left.0.total_cmp(&right.0));
        if state_neighbors
            .first()
            .is_some_and(|(distance, _)| *distance <= EXACT_STATE_DISTANCE_EPSILON)
        {
            state_neighbors.retain(|(distance, _)| *distance <= EXACT_STATE_DISTANCE_EPSILON);
        } else {
            state_neighbors.truncate(STATE_NEIGHBORS.min(state_neighbors.len()));
        }
        let mut neighbors = state_neighbors
            .into_iter()
            .map(|(state_distance, sample)| {
                (
                    state_distance
                        + normalized_distance(
                            &action,
                            &sample.action,
                            &self.action_min,
                            &self.action_range,
                        ) * 2.0,
                    sample,
                )
            })
            .collect::<Vec<_>>();
        neighbors.sort_by(|left, right| left.0.total_cmp(&right.0));
        neighbors.truncate(NEIGHBORS.min(neighbors.len()));
        let nearest_distance = neighbors[0].0;
        let mut outcome = GeneralizedTacticOutcome::default();
        let mut total_weight = 0.0_f32;
        let mut terminal_weight = 0.0_f32;
        let mut terminal_duration = 0.0_f32;
        for (distance, sample) in &neighbors {
            let weight = 1.0 / (0.01 + *distance);
            outcome.weighted_add(sample.outcome, weight);
            total_weight += weight;
            let supported_weight = weight * sample.outcome.terminal.clamp(0.0, 1.0);
            terminal_weight += supported_weight;
            terminal_duration += sample.outcome.duration_ticks * supported_weight;
        }
        outcome.scale(1.0 / total_weight);
        // First-hit cost is conditional on authenticated terminal support.
        // Censored rows calibrate reachability and expected return, but cannot
        // manufacture a zero-tick success.
        outcome.duration_ticks = if terminal_weight > 0.0 {
            terminal_duration / terminal_weight
        } else {
            0.0
        };
        Ok(GeneralizedTacticEstimate {
            descriptor: descriptor.clone(),
            outcome,
            nearest_distance,
            neighbors: neighbors.len(),
        })
    }

    pub fn rank(
        &self,
        state_features: &[f32],
        context: &GeneralizedTacticContext,
        descriptors: &[OptionActionDescriptor],
    ) -> Result<Vec<GeneralizedTacticEstimate>, GeneralizedTacticValueError> {
        let mut estimates = descriptors
            .iter()
            .map(|descriptor| self.predict(state_features, context, descriptor))
            .collect::<Result<Vec<_>, _>>()?;
        estimates.sort_by(|left, right| {
            compare_generalized_tactic_outcomes(&right.outcome, &left.outcome)
                .then_with(|| left.nearest_distance.total_cmp(&right.nearest_distance))
                .then_with(|| left.descriptor.option_id.cmp(&right.descriptor.option_id))
        });
        Ok(estimates)
    }
}

fn fitted_q_backup_limit(minimum_iterations: usize, transition_count: usize) -> usize {
    transition_count
        .max(minimum_iterations)
        .min(MAX_FITTED_Q_BACKUP_ITERATIONS)
}

fn terminal_supported_transition_indices(
    transitions: &[OptionTransitionSample],
) -> BTreeSet<usize> {
    let edges = transitions
        .iter()
        .map(|transition| {
            (
                transition.before_state_sha256,
                transition.after_state_sha256,
                transition.value_sample.terminal,
            )
        })
        .collect::<Vec<_>>();
    terminal_supported_edge_indices(&edges)
}

fn terminal_supported_edge_indices(edges: &[(Digest, Digest, bool)]) -> BTreeSet<usize> {
    let mut supported_states = BTreeSet::new();
    let mut supported_edges = BTreeSet::new();
    loop {
        let mut changed = false;
        for (index, (before, after, terminal)) in edges.iter().copied().enumerate() {
            if terminal || supported_states.contains(&after) {
                changed |= supported_edges.insert(index);
                changed |= supported_states.insert(before);
            }
        }
        if !changed {
            return supported_edges;
        }
    }
}

fn terminal_supported_first_hit_ticks(
    transitions: &[OptionTransitionSample],
    terminal_supported: &BTreeSet<usize>,
    iteration_limit: usize,
) -> Result<Vec<Option<u64>>, GeneralizedTacticValueError> {
    let mut outgoing = BTreeMap::<_, Vec<usize>>::new();
    for (index, transition) in transitions.iter().enumerate() {
        outgoing
            .entry(transition.before_state_sha256)
            .or_default()
            .push(index);
    }
    let mut ticks = transitions
        .iter()
        .map(|transition| {
            transition
                .value_sample
                .terminal
                .then_some(u64::from(transition.value_sample.duration_ticks))
        })
        .collect::<Vec<_>>();
    for _ in 0..iteration_limit.max(1) {
        let prior = ticks.clone();
        let mut changed = false;
        for (index, transition) in transitions.iter().enumerate() {
            if transition.value_sample.terminal || !terminal_supported.contains(&index) {
                continue;
            }
            let next_ticks = outgoing
                .get(&transition.after_state_sha256)
                .into_iter()
                .flatten()
                .filter(|next| terminal_supported.contains(next))
                .filter_map(|next| prior[*next])
                .min();
            let value = next_ticks
                .map(|next| next.saturating_add(u64::from(transition.value_sample.duration_ticks)));
            changed |= ticks[index] != value;
            ticks[index] = value;
        }
        if !changed {
            break;
        }
    }
    if terminal_supported
        .iter()
        .any(|index| ticks.get(*index).is_none_or(Option::is_none))
    {
        return Err(GeneralizedTacticValueError::InvalidTransition(
            "terminal-supported transition has no finite first-hit path".into(),
        ));
    }
    Ok(ticks)
}

pub fn compare_generalized_tactic_outcomes(
    left: &GeneralizedTacticOutcome,
    right: &GeneralizedTacticOutcome,
) -> std::cmp::Ordering {
    // `reward` is the learned objective return: authenticated terminal value
    // minus native input cost, including bootstrapped future value. Every other
    // outcome head is auxiliary evidence and cannot define policy utility.
    let bucket = |reward: f32| (f64::from(reward) / RETURN_COMPARISON_RESOLUTION).round();
    bucket(left.reward).total_cmp(&bucket(right.reward))
}

pub fn generalized_tactic_action_factors(
    context: &GeneralizedTacticContext,
    descriptor: &OptionActionDescriptor,
) -> Result<GeneralizedTacticActionFactors, GeneralizedTacticValueError> {
    let encoded = encode_action(context, descriptor)?;
    let button_mask = (0..16).fold(0_u16, |mask, bit| {
        mask | (u16::from(encoded[55 + bit] >= 0.5) << bit)
    });
    Ok(GeneralizedTacticActionFactors {
        planned_path_length: encoded[40],
        planned_displacement: encoded[41],
        planned_path_efficiency: encoded[42],
        planned_turn_radians: encoded[43],
        stick_magnitude: encoded[45],
        waypoint_switch_radius: encoded[46],
        button_active_fraction: encoded[51],
        button_pulse_count: encoded[52],
        button_mean_interval_ticks: encoded[53],
        button_phase_fraction: encoded[54],
        button_mask,
        rolling: button_mask & 0x0100 != 0 || descriptor.option_type == OptionType::Roll,
    })
}

fn encode_action(
    context: &GeneralizedTacticContext,
    descriptor: &OptionActionDescriptor,
) -> Result<[f32; GENERALIZED_TACTIC_ACTION_FEATURE_WIDTH], GeneralizedTacticValueError> {
    descriptor
        .validate()
        .map_err(|error| GeneralizedTacticValueError::InvalidAction(error.to_string()))?;
    let mut values = [0.0_f32; GENERALIZED_TACTIC_ACTION_FEATURE_WIDTH];
    values[option_type_index(&descriptor.option_type)] = 1.0;
    let mut cursor = 29;
    let duration = unsigned(descriptor, &["duration_ticks", "maximum_ticks"])
        .or_else(|| {
            unsigned(descriptor, &["recovery_frames"]).and_then(|frames| frames.checked_add(1))
        })
        .unwrap_or(1);
    values[cursor] = (duration as f32).ln_1p();
    cursor += 1;
    let coordinate_plan = coordinate_plan(descriptor);
    let first = pair(
        descriptor,
        "command_target_first_x",
        "command_target_first_z",
    )
    .or_else(|| coordinate_plan.first().copied());
    values[cursor] = f32::from(first.is_some());
    cursor += 1;
    if let Some([x, z]) = first {
        let dx = x - context.player_x;
        let dz = z - context.player_z;
        values[cursor] = dx;
        values[cursor + 1] = dz;
        values[cursor + 2] = dx.hypot(dz);
    }
    cursor += 3;
    let second = pair(
        descriptor,
        "command_target_second_x",
        "command_target_second_z",
    )
    .or_else(|| coordinate_plan.get(1).copied());
    values[cursor] = f32::from(second.is_some());
    cursor += 1;
    if let Some([x, z]) = second {
        values[cursor] = x - context.player_x;
        values[cursor + 1] = z - context.player_z;
    }
    cursor += 2;
    let last = pair(descriptor, "command_target_last_x", "command_target_last_z")
        .or_else(|| coordinate_plan.last().copied());
    if let Some([x, z]) = last {
        let dx = x - context.player_x;
        let dz = z - context.player_z;
        values[cursor] = dx;
        values[cursor + 1] = dz;
        values[cursor + 2] = dx.hypot(dz);
    }
    cursor += 3;
    let first_distance = first.map_or(0.0, |[x, z]| {
        (x - context.player_x).hypot(z - context.player_z)
    });
    let internal_path = float(descriptor, "command_internal_path_length").unwrap_or_else(|| {
        coordinate_plan
            .windows(2)
            .map(|pair| (pair[1][0] - pair[0][0]).hypot(pair[1][1] - pair[0][1]))
            .sum()
    });
    let planned_path = first_distance + internal_path;
    let planned_displacement = last.map_or(first_distance, |[x, z]| {
        (x - context.player_x).hypot(z - context.player_z)
    });
    values[cursor] = planned_path;
    values[cursor + 1] = planned_displacement;
    values[cursor + 2] = if planned_path > f32::EPSILON {
        (planned_displacement / planned_path).clamp(0.0, 1.0)
    } else {
        0.0
    };
    values[cursor + 3] = float(descriptor, "command_internal_turn_radians")
        .unwrap_or_else(|| plan_turn_radians(&coordinate_plan))
        + initial_turn(context, first, second);
    cursor += 4;
    values[cursor] = unsigned(descriptor, &["command_target_point_count", "target_count"])
        .unwrap_or(coordinate_plan.len() as u64) as f32;
    values[cursor + 1] = unsigned(
        descriptor,
        &["command_stick_magnitude", "magnitude", "movement_magnitude"],
    )
    .unwrap_or(0) as f32
        / 127.0;
    values[cursor + 2] = float(descriptor, "waypoint_switch_radius").unwrap_or(0.0);
    cursor += 3;
    let relative_heading = ["heading_radians", "movement_heading"]
        .iter()
        .find_map(|name| float(descriptor, name))
        .or_else(|| {
            float(descriptor, "direction_degrees")
                // RollOptionPlan defines positive degrees in raw-pad space,
                // whose X axis is opposite the world-heading convention used
                // by seek and maintained-heading controllers.
                .map(|degrees| -degrees * std::f32::consts::PI / 180.0)
        });
    values[cursor] = f32::from(relative_heading.is_some());
    if let Some(relative_heading) = relative_heading {
        let current_yaw = context.yaw_sin.atan2(context.yaw_cos);
        let camera_yaw = context.camera_yaw_sin.atan2(context.camera_yaw_cos);
        let desired_yaw = camera_yaw + relative_heading;
        values[cursor + 1] = desired_yaw.sin();
        values[cursor + 2] = desired_yaw.cos();
        values[cursor + 3] = angle_delta(desired_yaw, current_yaw).abs();
    }
    cursor += 4;
    let mask = unsigned(
        descriptor,
        &["command_button_mask", "button_pulse_mask", "button_mask"],
    )
    .unwrap_or_else(|| {
        if descriptor.option_type == OptionType::Roll {
            0x0100
        } else {
            0
        }
    }) as u16;
    values[cursor] = float(descriptor, "command_button_active_fraction")
        .unwrap_or_else(|| f32::from(mask != 0) / duration.max(1) as f32);
    values[cursor + 1] = unsigned(descriptor, &["command_button_pulse_count"])
        .unwrap_or(u64::from(mask != 0)) as f32;
    let interval = float(descriptor, "command_button_mean_interval_ticks")
        .or_else(|| {
            unsigned(descriptor, &["button_pulse_period_ticks", "period_ticks"]).map(|v| v as f32)
        })
        .unwrap_or(0.0);
    values[cursor + 2] = interval;
    let phase = unsigned(
        descriptor,
        &["button_pulse_phase_tick", "phase_tick", "button_frame"],
    )
    .unwrap_or(0) as f32;
    values[cursor + 3] = if interval > 0.0 {
        phase / interval
    } else {
        0.0
    };
    cursor += 4;
    for bit in 0..16 {
        values[cursor + bit] = f32::from((mask >> bit) & 1);
    }
    debug_assert_eq!(cursor + 16, GENERALIZED_TACTIC_ACTION_FEATURE_WIDTH);
    if values.iter().any(|value| !value.is_finite()) {
        return Err(GeneralizedTacticValueError::NonFinite);
    }
    Ok(values)
}

fn initial_turn(
    context: &GeneralizedTacticContext,
    first: Option<[f32; 2]>,
    second: Option<[f32; 2]>,
) -> f32 {
    let (Some(first), Some(second)) = (first, second) else {
        return 0.0;
    };
    let left = [first[0] - context.player_x, first[1] - context.player_z];
    let right = [second[0] - first[0], second[1] - first[1]];
    let cross = left[0] * right[1] - left[1] * right[0];
    let dot = left[0] * right[0] + left[1] * right[1];
    cross.atan2(dot).abs()
}

fn option_type_index(option_type: &OptionType) -> usize {
    match option_type {
        OptionType::Move
        | OptionType::Align
        | OptionType::MaintainHeading
        | OptionType::MaintainDistance
        | OptionType::Waypoint
        | OptionType::Rail
        | OptionType::Spline
        | OptionType::Bezier
        | OptionType::SeekActor
        | OptionType::MaintainOffset => 0,
        OptionType::Turn => 1,
        OptionType::Brake => 2,
        OptionType::Neutral => 3,
        OptionType::Roll => 7,
        OptionType::JumpAttack => 8,
        OptionType::Attack => 9,
        OptionType::Shield => 10,
        OptionType::Target => 11,
        OptionType::Interact => 12,
        OptionType::ItemUse => 13,
        OptionType::Transform => 14,
        OptionType::Crawl => 15,
        OptionType::Climb => 16,
        OptionType::Swim => 17,
        OptionType::Mount => 18,
        OptionType::Boomerang => 19,
        OptionType::Clawshot => 20,
        OptionType::Spinner => 21,
        OptionType::Custom(_) => 28,
    }
}

fn float(descriptor: &OptionActionDescriptor, name: &str) -> Option<f32> {
    descriptor
        .parameters
        .get(name)
        .and_then(|parameter| match parameter {
            OptionParameter::F32Bits(bits) => Some(f32::from_bits(*bits)),
            OptionParameter::Unsigned(value) => Some(*value as f32),
            OptionParameter::Signed(value) => Some(*value as f32),
            _ => None,
        })
}

fn unsigned(descriptor: &OptionActionDescriptor, names: &[&str]) -> Option<u64> {
    names.iter().find_map(|name| {
        descriptor
            .parameters
            .get(*name)
            .and_then(|parameter| match parameter {
                OptionParameter::Unsigned(value) => Some(*value),
                _ => None,
            })
    })
}

fn pair(descriptor: &OptionActionDescriptor, x: &str, z: &str) -> Option<[f32; 2]> {
    Some([float(descriptor, x)?, float(descriptor, z)?])
}

fn coordinate_plan(descriptor: &OptionActionDescriptor) -> Vec<[f32; 2]> {
    if let Some(OptionParameter::Vec3F32Bits(coordinate)) = descriptor.parameters.get("coordinate")
    {
        let point = [f32::from_bits(coordinate[0]), f32::from_bits(coordinate[2])];
        return point
            .iter()
            .all(|value| value.is_finite())
            .then_some(vec![point])
            .unwrap_or_default();
    }
    let Some(OptionParameter::Text(encoded)) = descriptor.parameters.get("coordinates") else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<[u32; 3]>>(encoded)
        .ok()
        .map(|coordinates| {
            coordinates
                .into_iter()
                .map(|coordinate| [f32::from_bits(coordinate[0]), f32::from_bits(coordinate[2])])
                .collect::<Vec<_>>()
        })
        .filter(|coordinates| {
            !coordinates.is_empty()
                && coordinates
                    .iter()
                    .flatten()
                    .all(|coordinate| coordinate.is_finite())
        })
        .unwrap_or_default()
}

fn plan_turn_radians(coordinates: &[[f32; 2]]) -> f32 {
    coordinates
        .windows(3)
        .map(|points| {
            let left = [points[1][0] - points[0][0], points[1][1] - points[0][1]];
            let right = [points[2][0] - points[1][0], points[2][1] - points[1][1]];
            let cross = left[0] * right[1] - left[1] * right[0];
            let dot = left[0] * right[0] + left[1] * right[1];
            cross.atan2(dot).abs()
        })
        .sum()
}

fn angle_delta(left: f32, right: f32) -> f32 {
    (left - right + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}

fn feature_ranges<'a>(rows: impl Iterator<Item = &'a [f32]>, width: usize) -> (Vec<f32>, Vec<f32>) {
    let mut minimum = vec![f32::INFINITY; width];
    let mut maximum = vec![f32::NEG_INFINITY; width];
    for row in rows {
        for (index, value) in row.iter().copied().enumerate() {
            minimum[index] = minimum[index].min(value);
            maximum[index] = maximum[index].max(value);
        }
    }
    let range = minimum
        .iter()
        .zip(&maximum)
        .map(|(minimum, maximum)| (maximum - minimum).max(1.0e-6))
        .collect();
    (minimum, range)
}

fn action_feature_ranges(
    samples: &[EncodedSample],
) -> (
    [f32; GENERALIZED_TACTIC_ACTION_FEATURE_WIDTH],
    [f32; GENERALIZED_TACTIC_ACTION_FEATURE_WIDTH],
) {
    let mut minimum = [f32::INFINITY; GENERALIZED_TACTIC_ACTION_FEATURE_WIDTH];
    let mut maximum = [f32::NEG_INFINITY; GENERALIZED_TACTIC_ACTION_FEATURE_WIDTH];
    for sample in samples {
        for (index, value) in sample.action.iter().copied().enumerate() {
            minimum[index] = minimum[index].min(value);
            maximum[index] = maximum[index].max(value);
        }
    }
    let range = std::array::from_fn(|index| (maximum[index] - minimum[index]).max(1.0e-6));
    (minimum, range)
}

fn normalized_distance(left: &[f32], right: &[f32], minimum: &[f32], range: &[f32]) -> f32 {
    let mut total = 0.0_f32;
    let mut active = 0_u32;
    for index in 0..left.len() {
        if range[index] <= 1.0e-6
            && (left[index] - minimum[index]).abs() <= 1.0e-6
            && (right[index] - minimum[index]).abs() <= 1.0e-6
        {
            continue;
        }
        let delta = (left[index] - right[index]) / range[index];
        total += delta.clamp(-4.0, 4.0).powi(2);
        active += 1;
    }
    if active == 0 {
        0.0
    } else {
        total / active as f32
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GeneralizedTacticValueError {
    SampleCount,
    FeatureWidth,
    InvalidConfig,
    NonFinite,
    InvalidFacts(String),
    InvalidAction(String),
    InvalidTransition(String),
}

impl fmt::Display for GeneralizedTacticValueError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SampleCount => formatter.write_str("generalized tactic sample count is invalid"),
            Self::FeatureWidth => {
                formatter.write_str("generalized tactic feature shape is invalid")
            }
            Self::InvalidConfig => {
                formatter.write_str("generalized tactic fitted-Q configuration is invalid")
            }
            Self::NonFinite => formatter.write_str("generalized tactic value is non-finite"),
            Self::InvalidFacts(message) => write!(formatter, "generalized tactic facts: {message}"),
            Self::InvalidAction(message) => {
                write!(formatter, "generalized tactic action: {message}")
            }
            Self::InvalidTransition(message) => {
                write!(formatter, "generalized tactic transition: {message}")
            }
        }
    }
}

impl Error for GeneralizedTacticValueError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tactic_asset::TacticAssetAdapter;
    use dusklight_control::tape::{InputFrame, InputTape};
    use std::collections::BTreeMap;

    fn action(
        id: &str,
        path_length: f32,
        displacement: f32,
        turn: f32,
        roll_period: Option<u64>,
        target_x: f32,
    ) -> OptionActionDescriptor {
        let mut parameters = BTreeMap::from([
            ("duration_ticks".into(), OptionParameter::Unsigned(160)),
            (
                "command_target_first_x".into(),
                OptionParameter::F32Bits(target_x.to_bits()),
            ),
            (
                "command_target_first_z".into(),
                OptionParameter::F32Bits(0.0_f32.to_bits()),
            ),
            (
                "command_target_last_x".into(),
                OptionParameter::F32Bits(displacement.to_bits()),
            ),
            (
                "command_target_last_z".into(),
                OptionParameter::F32Bits(0.0_f32.to_bits()),
            ),
            (
                "command_internal_path_length".into(),
                OptionParameter::F32Bits(path_length.to_bits()),
            ),
            (
                "command_internal_displacement".into(),
                OptionParameter::F32Bits(displacement.to_bits()),
            ),
            (
                "command_internal_turn_radians".into(),
                OptionParameter::F32Bits(turn.to_bits()),
            ),
            (
                "command_target_point_count".into(),
                OptionParameter::Unsigned(4),
            ),
            (
                "command_stick_magnitude".into(),
                OptionParameter::Unsigned(127),
            ),
        ]);
        if let Some(period) = roll_period {
            parameters.insert(
                "command_button_mask".into(),
                OptionParameter::Unsigned(0x0100),
            );
            parameters.insert(
                "button_pulse_period_ticks".into(),
                OptionParameter::Unsigned(period),
            );
            parameters.insert(
                "command_button_active_fraction".into(),
                OptionParameter::F32Bits((1.0 / period as f32).to_bits()),
            );
        }
        OptionActionDescriptor {
            option_id: id.into(),
            option_type: OptionType::Custom("reactive_controller".into()),
            parameters,
        }
    }

    fn sample(
        action: OptionActionDescriptor,
        reward: f32,
        outcome: GeneralizedTacticOutcome,
    ) -> GeneralizedTacticTrainingSample {
        GeneralizedTacticTrainingSample {
            state_features: vec![0.0, 1.0],
            context: GeneralizedTacticContext::default(),
            action,
            outcome: GeneralizedTacticOutcome { reward, ..outcome },
        }
    }

    #[test]
    fn terminal_support_excludes_censored_and_cyclic_components() {
        let digest = |byte| Digest([byte; 32]);
        let edges = [
            (digest(1), digest(2), false),
            (digest(2), digest(3), true),
            (digest(1), digest(4), false),
            (digest(4), digest(5), false),
            (digest(6), digest(2), false),
            (digest(7), digest(8), false),
            (digest(8), digest(7), false),
        ];

        assert_eq!(
            terminal_supported_edge_indices(&edges),
            BTreeSet::from([0, 1, 4])
        );
    }

    #[test]
    fn fitted_q_can_propagate_beyond_the_configured_minimum() {
        assert_eq!(fitted_q_backup_limit(12, 32), 32);
        assert_eq!(fitted_q_backup_limit(32, 4), 32);
        assert_eq!(fitted_q_backup_limit(12, 1_000), 512);
    }

    #[test]
    fn action_similarity_cannot_pull_value_from_a_remote_state_region() {
        let mut samples = Vec::new();
        for index in 0..STATE_NEIGHBORS {
            samples.push(GeneralizedTacticTrainingSample {
                state_features: vec![0.0, 0.0],
                context: GeneralizedTacticContext::default(),
                action: action(&format!("near-{index}"), 500.0, 10.0, 2.5, Some(7), -100.0),
                outcome: GeneralizedTacticOutcome {
                    reward: 1.0,
                    ..GeneralizedTacticOutcome::default()
                },
            });
        }
        let target = action("target", 100.0, 100.0, 0.0, None, 100.0);
        for index in 0..STATE_NEIGHBORS {
            let mut remote = target.clone();
            remote.option_id = format!("remote-{index}");
            samples.push(GeneralizedTacticTrainingSample {
                state_features: vec![100.0, 100.0],
                context: GeneralizedTacticContext::default(),
                action: remote,
                outcome: GeneralizedTacticOutcome {
                    reward: 100.0,
                    ..GeneralizedTacticOutcome::default()
                },
            });
        }

        let prediction = GeneralizedTacticValueModel::fit(&samples)
            .unwrap()
            .predict(&[0.0, 0.0], &GeneralizedTacticContext::default(), &target)
            .unwrap();
        assert_eq!(prediction.outcome.reward, 1.0);
    }

    #[test]
    fn later_action_similarity_cannot_override_exact_local_state_evidence() {
        let target = action("target", 100.0, 100.0, 0.0, None, 100.0);
        let mut samples = Vec::new();
        for index in 0..STATE_NEIGHBORS {
            samples.push(GeneralizedTacticTrainingSample {
                state_features: vec![0.0],
                context: GeneralizedTacticContext::default(),
                action: action(&format!("local-{index}"), 500.0, 10.0, 2.5, Some(7), -100.0),
                outcome: GeneralizedTacticOutcome {
                    reward: 1.0,
                    ..GeneralizedTacticOutcome::default()
                },
            });
        }
        for index in 0..STATE_NEIGHBORS {
            let mut later = target.clone();
            later.option_id = format!("later-{index}");
            samples.push(GeneralizedTacticTrainingSample {
                state_features: vec![0.25],
                context: GeneralizedTacticContext::default(),
                action: later,
                outcome: GeneralizedTacticOutcome {
                    reward: 100.0,
                    ..GeneralizedTacticOutcome::default()
                },
            });
        }

        let prediction = GeneralizedTacticValueModel::fit(&samples)
            .unwrap()
            .predict(&[0.0], &GeneralizedTacticContext::default(), &target)
            .unwrap();
        assert_eq!(prediction.outcome.reward, 1.0);
    }

    #[test]
    fn censored_neighbors_calibrate_expected_return_and_not_first_hit_cost() {
        let descriptor = action("query", 100.0, 100.0, 0.0, Some(20), 10.0);
        let mut samples = Vec::new();
        for index in 0..4 {
            let mut supported = descriptor.clone();
            supported.option_id = format!("supported-{index}");
            samples.push(GeneralizedTacticTrainingSample {
                state_features: vec![0.0, 1.0],
                context: GeneralizedTacticContext::default(),
                action: supported,
                outcome: GeneralizedTacticOutcome {
                    terminal: 1.0,
                    reward: 99.0,
                    duration_ticks: 10.0,
                    ..GeneralizedTacticOutcome::default()
                },
            });
            let mut censored = descriptor.clone();
            censored.option_id = format!("censored-{index}");
            samples.push(GeneralizedTacticTrainingSample {
                state_features: vec![0.0, 1.0],
                context: GeneralizedTacticContext::default(),
                action: censored,
                outcome: GeneralizedTacticOutcome {
                    terminal: 0.0,
                    reward: -1.0,
                    duration_ticks: 0.0,
                    ..GeneralizedTacticOutcome::default()
                },
            });
        }

        let prediction = GeneralizedTacticValueModel::fit(&samples)
            .unwrap()
            .predict(
                &[0.0, 1.0],
                &GeneralizedTacticContext::default(),
                &descriptor,
            )
            .unwrap();
        assert!((prediction.outcome.terminal - 0.5).abs() < 1.0e-6);
        assert!((prediction.outcome.reward - 49.0).abs() < 1.0e-6);
        assert_eq!(prediction.outcome.duration_ticks, 10.0);
    }

    #[test]
    fn action_identity_is_not_a_model_feature() {
        let mut left = action("left", 100.0, 100.0, 0.0, Some(20), 10.0);
        let mut right = action("right", 100.0, 100.0, 0.0, Some(20), 10.0);
        left.parameters.insert(
            "controller_sha256".into(),
            OptionParameter::Digest(crate::artifact::Digest([1; 32])),
        );
        right.parameters.insert(
            "controller_sha256".into(),
            OptionParameter::Digest(crate::artifact::Digest([2; 32])),
        );
        assert_eq!(
            encode_action(&GeneralizedTacticContext::default(), &left).unwrap(),
            encode_action(&GeneralizedTacticContext::default(), &right).unwrap()
        );
    }

    #[test]
    fn recorded_tape_identity_is_not_a_model_feature() {
        let mut frame = InputFrame::default();
        frame.owned_ports = 1;
        frame.pads[0].stick_y = 127;
        frame.pads[0].buttons = 0x0100;
        let left = InputTape {
            frames: vec![frame.clone(), InputFrame::default()],
            ..InputTape::default()
        };
        let right = InputTape {
            tick_rate_numerator: 60,
            frames: vec![frame, InputFrame::default()],
            ..InputTape::default()
        };
        let left = left.describe("recorded-left").unwrap().option;
        let right = right.describe("recorded-right").unwrap().option;
        assert_ne!(
            left.parameters.get("input_tape_sha256"),
            right.parameters.get("input_tape_sha256")
        );
        assert_eq!(
            encode_action(&GeneralizedTacticContext::default(), &left).unwrap(),
            encode_action(&GeneralizedTacticContext::default(), &right).unwrap()
        );
    }

    #[test]
    fn typed_native_targets_magnitude_and_heading_are_state_relative() {
        let context = GeneralizedTacticContext {
            player_x: 10.0,
            player_z: 20.0,
            yaw_cos: 1.0,
            camera_yaw_cos: 1.0,
            ..GeneralizedTacticContext::default()
        };
        let target = OptionActionDescriptor {
            option_id: "native-target".into(),
            option_type: OptionType::Move,
            parameters: BTreeMap::from([
                ("maximum_ticks".into(), OptionParameter::Unsigned(10)),
                (
                    "coordinate".into(),
                    OptionParameter::Vec3F32Bits([30.0_f32, 0.0, 20.0_f32].map(f32::to_bits)),
                ),
                ("magnitude".into(), OptionParameter::Unsigned(100)),
            ]),
        };
        let encoded_target = encode_action(&context, &target).unwrap();
        assert_eq!(encoded_target[30], 1.0);
        assert_eq!(&encoded_target[31..34], &[20.0, 0.0, 20.0]);
        assert_eq!(encoded_target[44], 1.0);
        assert_eq!(encoded_target[45], 100.0 / 127.0);

        let heading = OptionActionDescriptor {
            option_id: "native-heading".into(),
            option_type: OptionType::MaintainHeading,
            parameters: BTreeMap::from([
                ("maximum_ticks".into(), OptionParameter::Unsigned(10)),
                (
                    "heading_radians".into(),
                    OptionParameter::F32Bits(std::f32::consts::FRAC_PI_2.to_bits()),
                ),
                ("magnitude".into(), OptionParameter::Unsigned(127)),
            ]),
        };
        let encoded_heading = encode_action(&context, &heading).unwrap();
        assert_eq!(encoded_heading[0], 1.0);
        assert_eq!(encoded_heading[5], 0.0);
        assert_eq!(encoded_heading[47], 1.0);
        assert!((encoded_heading[48] - 1.0).abs() < 1.0e-6);
        assert!(encoded_heading[49].abs() < 1.0e-6);
        assert!((encoded_heading[50] - std::f32::consts::FRAC_PI_2).abs() < 1.0e-6);

        let roll = OptionActionDescriptor {
            option_id: "native-roll".into(),
            option_type: OptionType::Roll,
            parameters: BTreeMap::from([
                ("direction_degrees".into(), OptionParameter::Signed(-90)),
                ("magnitude".into(), OptionParameter::Unsigned(127)),
                ("recovery_frames".into(), OptionParameter::Unsigned(3)),
            ]),
        };
        let encoded_roll = encode_action(&context, &roll).unwrap();
        assert!((encoded_roll[29] - 4.0_f32.ln_1p()).abs() < 1.0e-6);
        assert_eq!(encoded_roll[47], 1.0);
        assert!((encoded_roll[48] - 1.0).abs() < 1.0e-6);
        assert!(encoded_roll[49].abs() < 1.0e-6);
        assert_eq!(encoded_roll[45], 1.0);
        assert!((encoded_roll[51] - 0.25).abs() < 1.0e-6);
        assert_eq!(encoded_roll[55 + 8], 1.0);
    }

    #[test]
    fn learned_objective_return_is_the_only_action_ordering() {
        let faster = GeneralizedTacticOutcome {
            terminal: 1.0,
            reward: 99.0,
            duration_ticks: 100.0,
            path_efficiency: 0.7,
            ..GeneralizedTacticOutcome::default()
        };
        let slower_but_cleaner = GeneralizedTacticOutcome {
            terminal: 1.0,
            reward: 98.0,
            duration_ticks: 110.0,
            path_efficiency: 1.0,
            ..GeneralizedTacticOutcome::default()
        };

        assert_eq!(
            compare_generalized_tactic_outcomes(&faster, &slower_but_cleaner),
            std::cmp::Ordering::Greater
        );
    }

    #[test]
    fn auxiliary_motion_predictions_do_not_define_action_utility() {
        let clean = GeneralizedTacticOutcome {
            goal_progress_per_tick: 20.0,
            path_efficiency: 0.98,
            speed_retention: 0.95,
            duration_ticks: 100.0,
            ..GeneralizedTacticOutcome::default()
        };
        let benign_clip = GeneralizedTacticOutcome {
            wall_contact_fraction: 0.5,
            ..clean
        };
        let slowing_impact = GeneralizedTacticOutcome {
            goal_progress_per_tick: 1.0,
            path_efficiency: 0.1,
            speed_retention: 0.1,
            stalled_command_fraction: 0.8,
            momentum_loss_per_tick: 1.0,
            collision_correction_per_tick: 0.5,
            ..benign_clip
        };

        assert_eq!(
            compare_generalized_tactic_outcomes(&benign_clip, &clean),
            std::cmp::Ordering::Equal
        );
        assert_eq!(
            compare_generalized_tactic_outcomes(&slowing_impact, &clean),
            std::cmp::Ordering::Equal
        );
    }

    #[test]
    fn subresolution_return_noise_defers_to_action_support_distance() {
        let reference = GeneralizedTacticOutcome {
            reward: 98.739_967,
            ..GeneralizedTacticOutcome::default()
        };
        let interpolation_noise = GeneralizedTacticOutcome {
            reward: 98.739_983,
            ..reference
        };
        let one_tick_gain = GeneralizedTacticOutcome {
            reward: reference.reward + 0.01,
            ..reference
        };

        assert_eq!(
            compare_generalized_tactic_outcomes(&interpolation_noise, &reference),
            std::cmp::Ordering::Equal
        );
        assert_eq!(
            compare_generalized_tactic_outcomes(&one_tick_gain, &reference),
            std::cmp::Ordering::Greater
        );
    }

    #[test]
    fn calibrated_return_comparison_is_transitive() {
        let outcomes = [
            98.739_90, 98.739_94, 98.739_967, 98.739_983, 98.740_02, 98.749_97,
        ]
        .map(|reward| GeneralizedTacticOutcome {
            reward,
            ..GeneralizedTacticOutcome::default()
        });

        for left in &outcomes {
            for middle in &outcomes {
                for right in &outcomes {
                    let left_middle = compare_generalized_tactic_outcomes(left, middle);
                    let middle_right = compare_generalized_tactic_outcomes(middle, right);
                    if left_middle != std::cmp::Ordering::Greater
                        && middle_right != std::cmp::Ordering::Greater
                    {
                        assert_ne!(
                            compare_generalized_tactic_outcomes(left, right),
                            std::cmp::Ordering::Greater
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn held_out_actions_generalize_roll_straightness_and_collision_outcomes() {
        let clean = GeneralizedTacticOutcome {
            terminal: 1.0,
            goal_progress_per_tick: 20.0,
            path_efficiency: 0.98,
            speed_retention: 0.95,
            ..GeneralizedTacticOutcome::default()
        };
        let curved = GeneralizedTacticOutcome {
            goal_progress_per_tick: 12.0,
            path_efficiency: 0.65,
            speed_retention: 0.7,
            ..GeneralizedTacticOutcome::default()
        };
        let wall = GeneralizedTacticOutcome {
            goal_progress_per_tick: 5.0,
            path_efficiency: 0.5,
            speed_retention: 0.3,
            wall_contact_fraction: 0.4,
            momentum_loss_per_tick: 3.0,
            collision_correction_per_tick: 2.0,
            ..GeneralizedTacticOutcome::default()
        };
        let samples = vec![
            sample(
                action("roll-18", 100.0, 100.0, 0.0, Some(18), 10.0),
                99.0,
                clean,
            ),
            sample(
                action("roll-22", 104.0, 100.0, 0.03, Some(22), 12.0),
                98.0,
                clean,
            ),
            sample(action("walk-a", 100.0, 100.0, 0.0, None, 10.0), 5.0, curved),
            sample(
                action("walk-b", 104.0, 100.0, 0.03, None, 12.0),
                5.0,
                curved,
            ),
            sample(
                action("curve-a", 150.0, 100.0, 1.2, Some(20), 10.0),
                20.0,
                curved,
            ),
            sample(
                action("curve-b", 145.0, 100.0, 1.0, Some(24), 12.0),
                20.0,
                curved,
            ),
            sample(
                action("wall-a", 120.0, 90.0, 0.4, Some(20), 90.0),
                -10.0,
                wall,
            ),
            sample(
                action("wall-b", 122.0, 90.0, 0.45, Some(22), 92.0),
                -9.0,
                wall,
            ),
        ];
        let model = GeneralizedTacticValueModel::fit(&samples).unwrap();
        let held_out = vec![
            action("held-roll", 102.0, 100.0, 0.01, Some(20), 11.0),
            action("held-walk", 102.0, 100.0, 0.01, None, 11.0),
            action("held-curve", 148.0, 100.0, 1.1, Some(21), 11.0),
            action("held-wall", 121.0, 90.0, 0.42, Some(21), 91.0),
        ];
        let ranked = model
            .rank(&[0.0, 1.0], &GeneralizedTacticContext::default(), &held_out)
            .unwrap();
        assert_eq!(ranked[0].descriptor.option_id, "held-roll");
        let by_id = |id: &str| {
            ranked
                .iter()
                .find(|estimate| estimate.descriptor.option_id == id)
                .unwrap()
        };
        assert!(by_id("held-roll").outcome.reward > by_id("held-walk").outcome.reward);
        assert!(by_id("held-roll").outcome.reward > by_id("held-curve").outcome.reward);
        assert!(
            by_id("held-wall").outcome.wall_contact_fraction
                > by_id("held-roll").outcome.wall_contact_fraction
        );
        assert!(by_id("held-wall").outcome.reward < by_id("held-roll").outcome.reward);
    }
}
