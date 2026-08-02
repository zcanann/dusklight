//! Minimal tabular semi-Markov learner for authenticated cold-start episodes.

use crate::artifact::Digest;
use crate::fact_snapshot::{FactSnapshot, PadFactSnapshot};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::f32::consts::{PI, TAU};
use std::fmt;

pub const SCRATCH_Q_SCHEMA_V1: &str = "dusklight-scratch-q/v1";
pub const SCRATCH_POSITION_CELL_WIDTH: f32 = 128.0;
pub const SCRATCH_VELOCITY_CELL_WIDTH: f32 = 2.0;
pub const SCRATCH_ANGLE_CELLS: u8 = 16;
pub const MAX_SCRATCH_ACTIONS: usize = 1_024;
pub const MAX_SCRATCH_EPISODE_TICKS: u32 = 100_000;
pub const EPSILON_SCALE: u32 = 1_000_000;

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScratchStateKey {
    pub position_xz: [i32; 2],
    pub velocity_xz: [i16; 2],
    pub facing: i8,
    pub camera: i8,
    pub prompted_action_status: u8,
    pub recent_input_headings: [i8; 2],
    pub recent_motion_headings: [i8; 2],
}

impl ScratchStateKey {
    pub fn from_snapshot(snapshot: &FactSnapshot) -> Result<Self, ScratchQError> {
        snapshot
            .validate()
            .map_err(|error| invalid(error.to_string()))?;
        let position = snapshot.player.position_f32_bits.map(f32::from_bits);
        let velocity = snapshot
            .player
            .velocity_f32_bits
            .map(|bits| bits.map(f32::from_bits))
            .unwrap_or([0.0; 3]);
        let facing = snapshot
            .player
            .current_angle
            .map(|angles| quantize_s16_angle(angles[1]))
            .unwrap_or(-1);
        let camera = snapshot
            .player
            .camera_yaw_radians_f32_bits
            .map(f32::from_bits)
            .map(quantize_radians)
            .unwrap_or(-1);
        let mut history_pads = snapshot
            .recent_history
            .iter()
            .rev()
            .take(2)
            .map(|history| history.previous_pad)
            .collect::<Vec<_>>();
        if history_pads.is_empty() {
            history_pads.extend(snapshot.player.previous_pad);
        }
        let mut recent_input_headings = [-1; 2];
        for (slot, pad) in recent_input_headings.iter_mut().zip(history_pads) {
            *slot = pad_heading(pad);
        }

        let mut points = snapshot
            .recent_history
            .iter()
            .rev()
            .take(2)
            .map(|history| history.player_position_f32_bits.map(f32::from_bits))
            .collect::<Vec<_>>();
        points.reverse();
        points.push(position);
        let mut recent_motion_headings = [-1; 2];
        for (slot, pair) in recent_motion_headings
            .iter_mut()
            .rev()
            .zip(points.windows(2).rev())
        {
            *slot = displacement_heading(pair[0], pair[1]);
        }

        Ok(Self {
            position_xz: [
                cell(position[0], SCRATCH_POSITION_CELL_WIDTH),
                cell(position[2], SCRATCH_POSITION_CELL_WIDTH),
            ],
            velocity_xz: [
                small_cell(velocity[0], SCRATCH_VELOCITY_CELL_WIDTH),
                small_cell(velocity[2], SCRATCH_VELOCITY_CELL_WIDTH),
            ],
            facing,
            camera,
            prompted_action_status: snapshot
                .player
                .action_state
                .map(|action| action.do_status)
                .unwrap_or(0),
            recent_input_headings,
            recent_motion_headings,
        })
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScratchQValue {
    pub mean_return: f64,
    pub visits: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScratchQTable {
    pub schema: String,
    pub action_count: usize,
    pub values: BTreeMap<ScratchStateAction, ScratchQValue>,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScratchStateAction {
    pub state: ScratchStateKey,
    pub action_index: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScratchTransition {
    pub state: ScratchStateKey,
    pub action_index: usize,
    pub realized_ticks: u32,
    pub next_state: ScratchStateKey,
    pub terminal: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScratchSelectionReason {
    Greedy,
    Epsilon,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScratchSelection {
    pub action_index: usize,
    pub reason: ScratchSelectionReason,
    pub draw: u32,
    pub q_value: f64,
    pub visits: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScratchUpdateSummary {
    pub updates: u64,
    pub changed_choices: u64,
}

impl ScratchQTable {
    pub fn new(action_count: usize) -> Result<Self, ScratchQError> {
        if action_count == 0 || action_count > MAX_SCRATCH_ACTIONS {
            return Err(invalid("scratch action count is invalid"));
        }
        Ok(Self {
            schema: SCRATCH_Q_SCHEMA_V1.into(),
            action_count,
            values: BTreeMap::new(),
        })
    }

    pub fn validate(&self) -> Result<(), ScratchQError> {
        if self.schema != SCRATCH_Q_SCHEMA_V1
            || self.action_count == 0
            || self.action_count > MAX_SCRATCH_ACTIONS
            || self.values.iter().any(|(key, value)| {
                key.action_index >= self.action_count
                    || value.visits == 0
                    || !value.mean_return.is_finite()
            })
        {
            return Err(invalid("scratch Q table is invalid"));
        }
        Ok(())
    }

    pub fn select(
        &self,
        state: &ScratchStateKey,
        eligible: &[usize],
        seed: u64,
        episode_index: u64,
        decision_index: u64,
        epsilon_per_million: u32,
    ) -> Result<ScratchSelection, ScratchQError> {
        self.validate()?;
        validate_eligible(eligible, self.action_count)?;
        if epsilon_per_million > EPSILON_SCALE {
            return Err(invalid("scratch epsilon is invalid"));
        }
        let draw = deterministic_draw(seed, episode_index, decision_index, state, b"epsilon")?;
        let explore = draw % EPSILON_SCALE < epsilon_per_million;
        let action_index = if explore {
            eligible[deterministic_draw(seed, episode_index, decision_index, state, b"action")?
                as usize
                % eligible.len()]
        } else {
            self.greedy_action(state, eligible)?
        };
        let value = self.values.get(&ScratchStateAction {
            state: state.clone(),
            action_index,
        });
        Ok(ScratchSelection {
            action_index,
            reason: if explore {
                ScratchSelectionReason::Epsilon
            } else {
                ScratchSelectionReason::Greedy
            },
            draw: draw % EPSILON_SCALE,
            q_value: value.map_or(0.0, |value| value.mean_return),
            visits: value.map_or(0, |value| value.visits),
        })
    }

    /// Applies one Monte-Carlo return pass. Successful episodes receive only
    /// the binary terminal value (`maximum_episode_ticks`); every realized
    /// native tick is then subtracted while walking backward. Censored episodes
    /// receive no terminal value.
    pub fn update_episode(
        &mut self,
        transitions: &[ScratchTransition],
        successful: bool,
        maximum_episode_ticks: u32,
    ) -> Result<ScratchUpdateSummary, ScratchQError> {
        self.validate()?;
        if transitions.is_empty()
            || maximum_episode_ticks == 0
            || maximum_episode_ticks > MAX_SCRATCH_EPISODE_TICKS
            || transitions.iter().any(|transition| {
                transition.action_index >= self.action_count || transition.realized_ticks == 0
            })
            || successful
                != transitions
                    .last()
                    .is_some_and(|transition| transition.terminal)
            || transitions[..transitions.len() - 1]
                .iter()
                .any(|transition| transition.terminal)
        {
            return Err(invalid("scratch episode is invalid"));
        }
        let touched = transitions
            .iter()
            .map(|transition| transition.state.clone())
            .collect::<BTreeSet<_>>();
        let all_actions = (0..self.action_count).collect::<Vec<_>>();
        let before = touched
            .iter()
            .map(|state| Ok((state.clone(), self.greedy_action(state, &all_actions)?)))
            .collect::<Result<BTreeMap<_, _>, ScratchQError>>()?;

        let mut return_ticks = if successful {
            f64::from(maximum_episode_ticks)
        } else {
            0.0
        };
        let mut updates = 0_u64;
        for transition in transitions.iter().rev() {
            return_ticks -= f64::from(transition.realized_ticks);
            let value = self
                .values
                .entry(ScratchStateAction {
                    state: transition.state.clone(),
                    action_index: transition.action_index,
                })
                .or_insert(ScratchQValue {
                    mean_return: 0.0,
                    visits: 0,
                });
            value.visits = value
                .visits
                .checked_add(1)
                .ok_or_else(|| invalid("scratch visit count overflowed"))?;
            value.mean_return += (return_ticks - value.mean_return) / value.visits as f64;
            updates += 1;
        }
        let changed_choices = touched
            .iter()
            .filter(|state| {
                self.greedy_action(state, &all_actions)
                    .is_ok_and(|after| before.get(*state).copied() != Some(after))
            })
            .count() as u64;
        self.validate()?;
        Ok(ScratchUpdateSummary {
            updates,
            changed_choices,
        })
    }

    pub fn unique_state_actions(&self) -> usize {
        self.values.len()
    }

    fn greedy_action(
        &self,
        state: &ScratchStateKey,
        eligible: &[usize],
    ) -> Result<usize, ScratchQError> {
        validate_eligible(eligible, self.action_count)?;
        eligible
            .iter()
            .copied()
            .max_by(|left, right| {
                let left_value = self
                    .values
                    .get(&ScratchStateAction {
                        state: state.clone(),
                        action_index: *left,
                    })
                    .map_or(0.0, |value| value.mean_return);
                let right_value = self
                    .values
                    .get(&ScratchStateAction {
                        state: state.clone(),
                        action_index: *right,
                    })
                    .map_or(0.0, |value| value.mean_return);
                left_value
                    .total_cmp(&right_value)
                    .then_with(|| right.cmp(left))
            })
            .ok_or_else(|| invalid("scratch action set is empty"))
    }
}

pub fn transition_sha256(transition: &ScratchTransition) -> Result<Digest, ScratchQError> {
    let bytes = serde_json::to_vec(&(
        &transition.state,
        transition.action_index,
        transition.realized_ticks,
        &transition.next_state,
        transition.terminal,
    ))
    .map_err(|error| invalid(error.to_string()))?;
    Ok(Digest(Sha256::digest(bytes).into()))
}

fn validate_eligible(eligible: &[usize], action_count: usize) -> Result<(), ScratchQError> {
    if eligible.is_empty()
        || eligible.iter().any(|action| *action >= action_count)
        || eligible.windows(2).any(|pair| pair[0] >= pair[1])
    {
        return Err(invalid("eligible scratch actions are invalid"));
    }
    Ok(())
}

fn deterministic_draw(
    seed: u64,
    episode_index: u64,
    decision_index: u64,
    state: &ScratchStateKey,
    domain: &[u8],
) -> Result<u32, ScratchQError> {
    let mut hasher = Sha256::new();
    hasher.update(SCRATCH_Q_SCHEMA_V1.as_bytes());
    hasher.update(domain);
    hasher.update(seed.to_le_bytes());
    hasher.update(episode_index.to_le_bytes());
    hasher.update(decision_index.to_le_bytes());
    hasher.update(serde_json::to_vec(state).map_err(|error| invalid(error.to_string()))?);
    Ok(u32::from_le_bytes(
        hasher.finalize()[..4].try_into().unwrap(),
    ))
}

fn cell(value: f32, width: f32) -> i32 {
    (value / width)
        .floor()
        .clamp(i32::MIN as f32, i32::MAX as f32) as i32
}

fn small_cell(value: f32, width: f32) -> i16 {
    (value / width).round().clamp(-32.0, 32.0) as i16
}

fn quantize_s16_angle(angle: i16) -> i8 {
    ((u32::from(angle as u16) * u32::from(SCRATCH_ANGLE_CELLS)) >> 16) as i8
}

fn quantize_radians(angle: f32) -> i8 {
    let normalized = (angle + PI).rem_euclid(TAU) / TAU;
    (normalized * f32::from(SCRATCH_ANGLE_CELLS)).floor() as i8
}

fn pad_heading(pad: PadFactSnapshot) -> i8 {
    let magnitude = f32::from(pad.stick_x).hypot(f32::from(pad.stick_y));
    if magnitude < 16.0 {
        -1
    } else {
        quantize_radians((-f32::from(pad.stick_x)).atan2(f32::from(pad.stick_y)))
    }
}

fn displacement_heading(before: [f32; 3], after: [f32; 3]) -> i8 {
    let x = after[0] - before[0];
    let z = after[2] - before[2];
    if x.hypot(z) < 1.0 {
        -1
    } else {
        quantize_radians(x.atan2(z))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScratchQError(String);

impl fmt::Display for ScratchQError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for ScratchQError {}

fn invalid(message: impl Into<String>) -> ScratchQError {
    ScratchQError(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(position: i32) -> ScratchStateKey {
        ScratchStateKey {
            position_xz: [position, 0],
            velocity_xz: [0, 0],
            facing: 0,
            camera: 0,
            prompted_action_status: 0,
            recent_input_headings: [-1; 2],
            recent_motion_headings: [-1; 2],
        }
    }

    #[test]
    fn selection_is_deterministic_and_updates_affect_the_next_choice() {
        let mut table = ScratchQTable::new(3).unwrap();
        let start = state(0);
        assert_eq!(
            table
                .select(&start, &[0, 1, 2], 7, 0, 0, 0)
                .unwrap()
                .action_index,
            0
        );
        let summary = table
            .update_episode(
                &[ScratchTransition {
                    state: start.clone(),
                    action_index: 1,
                    realized_ticks: 10,
                    next_state: state(1),
                    terminal: true,
                }],
                true,
                900,
            )
            .unwrap();
        assert_eq!(summary.updates, 1);
        assert_eq!(summary.changed_choices, 1);
        assert_eq!(
            table
                .select(&start, &[0, 1, 2], 7, 1, 0, 0)
                .unwrap()
                .action_index,
            1
        );
    }

    #[test]
    fn shorter_success_has_greater_return_and_failure_is_worse() {
        let mut table = ScratchQTable::new(3).unwrap();
        let start = state(0);
        for (action_index, ticks, success) in [(0, 125, true), (1, 124, true), (2, 900, false)] {
            table
                .update_episode(
                    &[ScratchTransition {
                        state: start.clone(),
                        action_index,
                        realized_ticks: ticks,
                        next_state: state(action_index as i32 + 1),
                        terminal: success,
                    }],
                    success,
                    900,
                )
                .unwrap();
        }
        assert_eq!(table.greedy_action(&start, &[0, 1, 2]).unwrap(), 1);
        assert!(
            table.values[&ScratchStateAction {
                state: start.clone(),
                action_index: 1
            }]
                .mean_return
                > table.values[&ScratchStateAction {
                    state: start.clone(),
                    action_index: 0
                }]
                    .mean_return
        );
        assert!(
            table.values[&ScratchStateAction {
                state: start,
                action_index: 2
            }]
                .mean_return
                < 0.0
        );
    }

    #[test]
    fn intermediate_option_ticks_are_charged_once_in_backward_order() {
        let mut table = ScratchQTable::new(2).unwrap();
        table
            .update_episode(
                &[
                    ScratchTransition {
                        state: state(0),
                        action_index: 0,
                        realized_ticks: 10,
                        next_state: state(1),
                        terminal: false,
                    },
                    ScratchTransition {
                        state: state(1),
                        action_index: 1,
                        realized_ticks: 15,
                        next_state: state(2),
                        terminal: true,
                    },
                ],
                true,
                100,
            )
            .unwrap();
        assert_eq!(
            table.values[&ScratchStateAction {
                state: state(1),
                action_index: 1
            }]
                .mean_return,
            85.0
        );
        assert_eq!(
            table.values[&ScratchStateAction {
                state: state(0),
                action_index: 0
            }]
                .mean_return,
            75.0
        );
    }

    #[test]
    fn equal_horizon_failures_cannot_rank_different_starting_actions() {
        let mut table = ScratchQTable::new(2).unwrap();
        let start = state(0);
        for (action_index, midpoint) in [(0, state(100)), (1, state(-100))] {
            table
                .update_episode(
                    &[
                        ScratchTransition {
                            state: start.clone(),
                            action_index,
                            realized_ticks: 400,
                            next_state: midpoint.clone(),
                            terminal: false,
                        },
                        ScratchTransition {
                            state: midpoint,
                            action_index,
                            realized_ticks: 500,
                            next_state: state(action_index as i32 + 1),
                            terminal: false,
                        },
                    ],
                    false,
                    900,
                )
                .unwrap();
        }
        let first = table.values[&ScratchStateAction {
            state: start.clone(),
            action_index: 0,
        }]
            .mean_return;
        let second = table.values[&ScratchStateAction {
            state: start,
            action_index: 1,
        }]
            .mean_return;
        assert_eq!(first, -900.0);
        assert_eq!(second, -900.0);
    }
}
