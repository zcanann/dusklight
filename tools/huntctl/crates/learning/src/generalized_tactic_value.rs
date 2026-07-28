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

mod fitted_q;
mod prediction;

pub const GENERALIZED_TACTIC_ACTION_FEATURE_WIDTH: usize = 71;
const GENERALIZED_TACTIC_BEHAVIOR_CONTEXT_WIDTH: usize = 12;
const MAX_GENERALIZED_TACTIC_SAMPLES: usize = 100_000;
const MAX_FITTED_Q_BACKUP_ITERATIONS: usize = 512;
const NEIGHBORS: usize = 8;
const STATE_NEIGHBORS: usize = 16;
const EXACT_STATE_DISTANCE_EPSILON: f32 = 1.0e-8;
const MINIMUM_RETURN_COMPARISON_RESOLUTION: f64 = 1.0e-4;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GeneralizedTacticContext {
    pub simulation_tick: f32,
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
            simulation_tick: facts.simulation_tick as f32,
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

    fn values(self) -> [f32; GENERALIZED_TACTIC_BEHAVIOR_CONTEXT_WIDTH] {
        [
            self.simulation_tick,
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
    pub terminal_support_distance: Option<f32>,
    pub neighbors: usize,
}

#[derive(Clone, Debug)]
struct EncodedSample {
    state: Vec<f32>,
    behavior_context: [f32; GENERALIZED_TACTIC_BEHAVIOR_CONTEXT_WIDTH],
    action: [f32; GENERALIZED_TACTIC_ACTION_FEATURE_WIDTH],
    outcome: GeneralizedTacticOutcome,
}

#[derive(Clone, Debug)]
pub struct GeneralizedTacticValueModel {
    state_min: Vec<f32>,
    state_range: Vec<f32>,
    behavior_context_min: [f32; GENERALIZED_TACTIC_BEHAVIOR_CONTEXT_WIDTH],
    behavior_context_range: [f32; GENERALIZED_TACTIC_BEHAVIOR_CONTEXT_WIDTH],
    action_min: [f32; GENERALIZED_TACTIC_ACTION_FEATURE_WIDTH],
    action_range: [f32; GENERALIZED_TACTIC_ACTION_FEATURE_WIDTH],
    return_comparison_resolution: f64,
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
        let mut model = Self::fit(&samples)?;
        model.return_comparison_resolution = observed_return_resolution(transitions);
        Ok(model)
    }

    /// Fits a shared semi-Markov action-value model.
    ///
    /// Bellman backups use only the authenticated scalar reward and observed
    /// successor actions. When exact hashes do not reconnect, typed-compatible
    /// nearest observed states supply an approximate Bellman continuation.
    /// This shares continuous-state value without claiming an exact replay edge:
    /// terminal support and first-hit ticks remain exact-only evidence.
    /// Auxiliary trajectory outcomes remain inspection evidence and cannot
    /// enter objective return or action ordering.
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
        let fitted_q =
            fitted_q::fit_transition_returns(transitions, iterations, per_tick_discount)?;
        for (index, (sample, value)) in samples.iter_mut().zip(fitted_q.values).enumerate() {
            sample.outcome.reward = value;
            sample.outcome.terminal = f32::from(fitted_q.exact_terminal_supported.contains(&index));
            sample.outcome.duration_ticks =
                fitted_q.exact_first_hit_ticks[index].unwrap_or(0) as f32;
        }
        let mut model = Self::fit(&samples)?;
        model.return_comparison_resolution = observed_return_resolution(transitions);
        Ok(model)
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
                    behavior_context: sample.context.values(),
                    action: encode_action(&sample.context, &sample.action)?,
                    outcome: sample.outcome,
                })
            })
            .collect::<Result<Vec<_>, GeneralizedTacticValueError>>()?;
        let (state_min, state_range) = feature_ranges(
            encoded.iter().map(|sample| sample.state.as_slice()),
            state_width,
        );
        let (behavior_context_min, behavior_context_range) =
            fixed_feature_ranges(encoded.iter().map(|sample| &sample.behavior_context));
        let (action_min, action_range) = action_feature_ranges(&encoded);
        Ok(Self {
            state_min,
            state_range,
            behavior_context_min,
            behavior_context_range,
            action_min,
            action_range,
            return_comparison_resolution: MINIMUM_RETURN_COMPARISON_RESOLUTION,
            samples: encoded,
        })
    }

    pub fn predict(
        &self,
        state_features: &[f32],
        context: &GeneralizedTacticContext,
        descriptor: &OptionActionDescriptor,
    ) -> Result<GeneralizedTacticEstimate, GeneralizedTacticValueError> {
        prediction::estimate_actions(
            self,
            state_features,
            context,
            std::slice::from_ref(descriptor),
        )
        .map(|mut estimates| {
            estimates
                .pop()
                .expect("one descriptor produces one generalized estimate")
        })
    }

    pub fn rank(
        &self,
        state_features: &[f32],
        context: &GeneralizedTacticContext,
        descriptors: &[OptionActionDescriptor],
    ) -> Result<Vec<GeneralizedTacticEstimate>, GeneralizedTacticValueError> {
        let mut estimates =
            prediction::estimate_actions(self, state_features, context, descriptors)?;
        estimates.sort_by(|left, right| {
            compare_generalized_tactic_estimates(left, right, self.return_comparison_resolution)
        });
        Ok(estimates)
    }

    /// Ranks executable actions as a behavior-cloning policy over the nearest
    /// authenticated terminal-supported trajectory phase and physical state.
    ///
    /// This is a separate acquisition lane, not a reward bonus. Other lanes
    /// remain Q-ranked, so a demonstration supplies a reproducible foothold
    /// without becoming the only policy or constraining later improvement.
    pub fn rank_terminal_support(
        &self,
        state_features: &[f32],
        context: &GeneralizedTacticContext,
        descriptors: &[OptionActionDescriptor],
    ) -> Result<Vec<GeneralizedTacticEstimate>, GeneralizedTacticValueError> {
        let mut estimates =
            prediction::estimate_actions(self, state_features, context, descriptors)?;
        estimates.sort_by(|left, right| {
            compare_terminal_support_estimates(left, right, self.return_comparison_resolution)
        });
        Ok(estimates)
    }
}

fn observed_return_resolution(transitions: &[OptionTransitionSample]) -> f64 {
    transitions
        .iter()
        .filter(|transition| !transition.value_sample.terminal)
        .filter_map(|transition| {
            let duration = f64::from(transition.value_sample.duration_ticks);
            let per_tick = f64::from(transition.value_sample.reward).abs() / duration;
            (per_tick.is_finite() && per_tick > MINIMUM_RETURN_COMPARISON_RESOLUTION)
                .then_some(per_tick)
        })
        .min_by(f64::total_cmp)
        .unwrap_or(MINIMUM_RETURN_COMPARISON_RESOLUTION)
        .max(MINIMUM_RETURN_COMPARISON_RESOLUTION)
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
    compare_generalized_tactic_outcomes_with_resolution(
        left,
        right,
        MINIMUM_RETURN_COMPARISON_RESOLUTION,
    )
}

fn compare_generalized_tactic_outcomes_with_resolution(
    left: &GeneralizedTacticOutcome,
    right: &GeneralizedTacticOutcome,
    resolution: f64,
) -> std::cmp::Ordering {
    // `reward` is the learned objective return: authenticated terminal value
    // minus native input cost, including bootstrapped future value. Every other
    // outcome head is auxiliary evidence and cannot define policy utility.
    let bucket = |reward: f32| (f64::from(reward) / resolution).round();
    bucket(left.reward).total_cmp(&bucket(right.reward))
}

fn compare_generalized_tactic_estimates(
    left: &GeneralizedTacticEstimate,
    right: &GeneralizedTacticEstimate,
    return_resolution: f64,
) -> std::cmp::Ordering {
    compare_generalized_tactic_outcomes_with_resolution(
        &right.outcome,
        &left.outcome,
        return_resolution,
    )
    .then_with(|| {
        match (
            left.terminal_support_distance,
            right.terminal_support_distance,
        ) {
            (Some(left), Some(right)) => left.total_cmp(&right),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        }
    })
    .then_with(|| left.nearest_distance.total_cmp(&right.nearest_distance))
    .then_with(|| left.descriptor.option_id.cmp(&right.descriptor.option_id))
}

fn compare_terminal_support_estimates(
    left: &GeneralizedTacticEstimate,
    right: &GeneralizedTacticEstimate,
    return_resolution: f64,
) -> std::cmp::Ordering {
    match (
        left.terminal_support_distance,
        right.terminal_support_distance,
    ) {
        (Some(left), Some(right)) => left.total_cmp(&right),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }
    .then_with(|| {
        compare_generalized_tactic_outcomes_with_resolution(
            &right.outcome,
            &left.outcome,
            return_resolution,
        )
    })
    .then_with(|| left.nearest_distance.total_cmp(&right.nearest_distance))
    .then_with(|| left.descriptor.option_id.cmp(&right.descriptor.option_id))
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
    // Compare what the controller actually emits, rather than comparing
    // similarly named parameters with different coordinate-frame semantics.
    // Recorded chunks expose their raw PAD heading directly. Roll headings use
    // the opposite X convention. Native maintained-heading plans compile a
    // camera-relative authored offset into a player-frame controller, whose
    // first emitted PAD heading is player - authored - camera.
    let current_yaw = context.yaw_sin.atan2(context.yaw_cos);
    let camera_yaw = context.camera_yaw_sin.atan2(context.camera_yaw_cos);
    let emitted_heading = float(descriptor, "command_initial_heading")
        .or_else(|| float(descriptor, "movement_heading"))
        .or_else(|| {
            float(descriptor, "heading_radians")
                .map(|authored| angle_delta(current_yaw - authored, camera_yaw))
        })
        .or_else(|| {
            float(descriptor, "direction_degrees")
                .map(|degrees| -degrees * std::f32::consts::PI / 180.0)
        });
    values[cursor] = f32::from(emitted_heading.is_some());
    if let Some(emitted_heading) = emitted_heading {
        values[cursor + 1] = emitted_heading.sin();
        values[cursor + 2] = emitted_heading.cos();
        values[cursor + 3] = emitted_heading.abs();
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

fn behavior_cloning_action_distance(
    left: &[f32],
    right: &[f32],
    minimum: &[f32],
    range: &[f32],
) -> f32 {
    let continuous = normalized_distance(left, right, minimum, range);
    const BUTTON_L_BIT: usize = 6;
    let prompted_action_mismatches = (55..71)
        // L targeting is a movement/camera modifier: it changes how an
        // otherwise comparable direction is realized, rather than expressing
        // a prompted world action. Keep its ordinary continuous distance so
        // native outcomes can distinguish it, but allow a demonstrated
        // movement heading to transfer to the corresponding camera-lock
        // tactic. A, B, and future prompt buttons remain categorical.
        .filter(|index| *index != 55 + BUTTON_L_BIT)
        .filter(|index| (left[*index] >= 0.5) != (right[*index] >= 0.5))
        .count() as f32;
    // A prompted action (roll today; jump, mount, lift, or another button in
    // future catalogs) is a categorical behavioral choice. Make one mismatch
    // dominate every possible continuous-factor difference without turning it
    // into reward or terminal evidence.
    continuous + prompted_action_mismatches * (GENERALIZED_TACTIC_ACTION_FEATURE_WIDTH as f32 + 1.0)
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

fn fixed_feature_ranges<'a, const WIDTH: usize>(
    rows: impl Iterator<Item = &'a [f32; WIDTH]>,
) -> ([f32; WIDTH], [f32; WIDTH]) {
    let mut minimum = [f32::INFINITY; WIDTH];
    let mut maximum = [f32::NEG_INFINITY; WIDTH];
    for row in rows {
        for (index, value) in row.iter().copied().enumerate() {
            minimum[index] = minimum[index].min(value);
            maximum[index] = maximum[index].max(value);
        }
    }
    let range = std::array::from_fn(|index| {
        let width = maximum[index] - minimum[index];
        if width > f32::EPSILON { width } else { 1.0 }
    });
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
mod synthetic_control_tests;
#[cfg(test)]
mod tests;
