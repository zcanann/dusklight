//! Generic, phase-correct temporal indexing for authenticated native episodes.
//!
//! The source shard and its derived actor/collision/geometry views already own
//! the actual observations. This artifact gives every model-facing view one
//! canonical episode-local history without copying those large states. A
//! decision can resolve its current pre-input observation and only transitions
//! which completed before that decision. The current transition's post-state
//! remains a separate training target.

use crate::artifact::Digest;
use dusklight_evidence::native_episode_shard::{
    NativeEpisodeShard, NativeRawPad, NativeTerminalReason,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;

pub const NATIVE_EPISODE_HISTORY_SCHEMA_V1: &str = "dusklight-native-episode-history/v1";
pub const DEFAULT_EPISODE_HISTORY_DEPTH: usize = 8;
pub const MAX_EPISODE_HISTORY_DEPTH: usize = 64;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EpisodeHistoryPhase {
    PreInput,
    PostSimulation,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EpisodeHistoryTerminalReason {
    None,
    GoalReached,
    TickBudgetExhausted,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EpisodeHistoryPad {
    pub buttons: u16,
    pub stick_x: i8,
    pub stick_y: i8,
    pub substick_x: i8,
    pub substick_y: i8,
    pub trigger_left: u8,
    pub trigger_right: u8,
    pub analog_a: u8,
    pub analog_b: u8,
    pub connected: bool,
    pub error: i8,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EpisodeHistoryBoundary {
    pub episode_id: String,
    pub step_index: u32,
    pub phase: EpisodeHistoryPhase,
    /// Exact ordinal used by native actor/geometry/collision views: every
    /// source step contributes pre-input then post-simulation.
    pub source_observation_ordinal: u32,
    pub boundary_index: u64,
    pub simulation_tick: u64,
    pub tape_frame: u64,
    pub state_identity_xxh3_128: String,
    pub stage: String,
    pub room: i8,
    pub layer: i8,
    pub point: i16,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EpisodeHistoryTransition {
    pub episode_id: String,
    pub step_index: u32,
    pub before_boundary_index: u32,
    pub after_boundary_index: u32,
    pub chosen_pad: EpisodeHistoryPad,
    pub consumed_pad: EpisodeHistoryPad,
    pub terminal_reason: EpisodeHistoryTerminalReason,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EpisodeHistoryDecision {
    pub episode_id: String,
    pub step_index: u32,
    pub current_boundary_index: u32,
    /// Oldest to newest. Every index names a transition completed strictly
    /// before this decision and within this episode.
    pub completed_transition_indices: Vec<u32>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeEpisodeHistoryView {
    pub schema: String,
    pub native_shard_sha256: Digest,
    pub observation_schema: String,
    pub action_schema: String,
    pub history_depth: usize,
    pub source_observation_count: u32,
    pub boundaries: Vec<EpisodeHistoryBoundary>,
    pub decisions: Vec<EpisodeHistoryDecision>,
    /// Training targets and realized actions. A caller must never append the
    /// current entry to its corresponding decision input.
    pub transitions: Vec<EpisodeHistoryTransition>,
    pub view_sha256: Digest,
}

impl NativeEpisodeHistoryView {
    pub fn build(
        shard: &NativeEpisodeShard,
        history_depth: usize,
    ) -> Result<Self, NativeEpisodeHistoryError> {
        if shard.content_sha256 == Digest::ZERO
            || shard.episodes.is_empty()
            || !(1..=MAX_EPISODE_HISTORY_DEPTH).contains(&history_depth)
        {
            return Err(NativeEpisodeHistoryError::new(
                "episode history requires an authenticated shard and bounded nonzero depth",
            ));
        }

        let step_count = shard
            .episodes
            .iter()
            .try_fold(0_usize, |count, episode| {
                count.checked_add(episode.steps.len())
            })
            .ok_or_else(|| NativeEpisodeHistoryError::new("episode step count overflowed"))?;
        if step_count == 0 || step_count > u32::MAX as usize / 2 {
            return Err(NativeEpisodeHistoryError::new(
                "episode history source step count is invalid",
            ));
        }

        let mut boundaries = Vec::with_capacity(step_count * 2);
        let mut decisions = Vec::with_capacity(step_count);
        let mut transitions = Vec::with_capacity(step_count);
        for episode in &shard.episodes {
            if episode.steps.is_empty() {
                return Err(NativeEpisodeHistoryError::new(
                    "episode history source contains an empty episode",
                ));
            }
            let mut completed = Vec::<u32>::new();
            for (step_index, step) in episode.steps.iter().enumerate() {
                let step_index = u32::try_from(step_index)
                    .map_err(|_| NativeEpisodeHistoryError::new("step index overflowed"))?;
                let pre_index = u32::try_from(boundaries.len())
                    .map_err(|_| NativeEpisodeHistoryError::new("boundary index overflowed"))?;
                boundaries.push(boundary(
                    &episode.id,
                    step_index,
                    pre_index,
                    EpisodeHistoryPhase::PreInput,
                    step.pre_input.boundary_index,
                    step.pre_input.simulation_tick,
                    step.pre_input.tape_frame,
                    step.pre_input.state_identity,
                    &step.pre_input.stage,
                    step.pre_input.room,
                    step.pre_input.layer,
                    step.pre_input.point,
                ));
                let post_index = u32::try_from(boundaries.len())
                    .map_err(|_| NativeEpisodeHistoryError::new("boundary index overflowed"))?;
                boundaries.push(boundary(
                    &episode.id,
                    step_index,
                    post_index,
                    EpisodeHistoryPhase::PostSimulation,
                    step.post_simulation.boundary_index,
                    step.post_simulation.simulation_tick,
                    step.post_simulation.tape_frame,
                    step.post_simulation.state_identity,
                    &step.post_simulation.stage,
                    step.post_simulation.room,
                    step.post_simulation.layer,
                    step.post_simulation.point,
                ));

                let retained_from = completed.len().saturating_sub(history_depth);
                decisions.push(EpisodeHistoryDecision {
                    episode_id: episode.id.clone(),
                    step_index,
                    current_boundary_index: pre_index,
                    completed_transition_indices: completed[retained_from..].to_vec(),
                });
                let transition_index = u32::try_from(transitions.len())
                    .map_err(|_| NativeEpisodeHistoryError::new("transition index overflowed"))?;
                transitions.push(EpisodeHistoryTransition {
                    episode_id: episode.id.clone(),
                    step_index,
                    before_boundary_index: pre_index,
                    after_boundary_index: post_index,
                    chosen_pad: pad(step.chosen_pad),
                    consumed_pad: pad(step.consumed_pad),
                    terminal_reason: terminal_reason(step.post_simulation.terminal_reason),
                });
                completed.push(transition_index);
            }
        }

        let mut view = Self {
            schema: NATIVE_EPISODE_HISTORY_SCHEMA_V1.into(),
            native_shard_sha256: shard.content_sha256,
            observation_schema: shard.metadata.observation_schema.clone(),
            action_schema: shard.metadata.action_schema.clone(),
            history_depth,
            source_observation_count: u32::try_from(boundaries.len())
                .map_err(|_| NativeEpisodeHistoryError::new("observation count overflowed"))?,
            boundaries,
            decisions,
            transitions,
            view_sha256: Digest::ZERO,
        };
        view.view_sha256 = view.compute_identity()?;
        view.validate()?;
        Ok(view)
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, NativeEpisodeHistoryError> {
        self.validate()?;
        serde_json::to_vec(self).map_err(|error| NativeEpisodeHistoryError::new(error.to_string()))
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, NativeEpisodeHistoryError> {
        let view: Self = serde_json::from_slice(bytes)
            .map_err(|error| NativeEpisodeHistoryError::new(error.to_string()))?;
        view.validate()?;
        if view.canonical_bytes()? != bytes {
            return Err(NativeEpisodeHistoryError::new(
                "episode history bytes are not canonical",
            ));
        }
        Ok(view)
    }

    pub fn current_boundary(
        &self,
        decision_index: usize,
    ) -> Result<&EpisodeHistoryBoundary, NativeEpisodeHistoryError> {
        self.validate()?;
        let decision = self.decisions.get(decision_index).ok_or_else(|| {
            NativeEpisodeHistoryError::new("episode history decision index is out of range")
        })?;
        Ok(&self.boundaries[decision.current_boundary_index as usize])
    }

    pub fn resolved_history(
        &self,
        decision_index: usize,
    ) -> Result<Vec<&EpisodeHistoryTransition>, NativeEpisodeHistoryError> {
        self.validate()?;
        let decision = self.decisions.get(decision_index).ok_or_else(|| {
            NativeEpisodeHistoryError::new("episode history decision index is out of range")
        })?;
        Ok(decision
            .completed_transition_indices
            .iter()
            .map(|index| &self.transitions[*index as usize])
            .collect())
    }

    pub fn validate(&self) -> Result<(), NativeEpisodeHistoryError> {
        self.validate_content()?;
        if self.view_sha256 != self.compute_identity()? {
            return Err(NativeEpisodeHistoryError::new(
                "episode history seal is invalid",
            ));
        }
        Ok(())
    }

    fn validate_content(&self) -> Result<(), NativeEpisodeHistoryError> {
        if self.schema != NATIVE_EPISODE_HISTORY_SCHEMA_V1
            || self.native_shard_sha256 == Digest::ZERO
            || self.observation_schema.is_empty()
            || self.action_schema.is_empty()
            || !(1..=MAX_EPISODE_HISTORY_DEPTH).contains(&self.history_depth)
            || self.decisions.is_empty()
            || self.decisions.len() != self.transitions.len()
            || self.boundaries.len() != self.transitions.len() * 2
            || self.source_observation_count as usize != self.boundaries.len()
        {
            return Err(NativeEpisodeHistoryError::new(
                "episode history envelope or cardinality is invalid",
            ));
        }

        for (index, boundary) in self.boundaries.iter().enumerate() {
            if boundary.episode_id.is_empty()
                || boundary.source_observation_ordinal as usize != index
                || boundary.state_identity_xxh3_128.len() != 32
                || !boundary
                    .state_identity_xxh3_128
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
                || boundary.stage.is_empty()
            {
                return Err(NativeEpisodeHistoryError::new(
                    "episode history boundary is invalid",
                ));
            }
        }

        let mut current_episode = None::<&str>;
        let mut completed = Vec::<u32>::new();
        for (index, (decision, transition)) in
            self.decisions.iter().zip(&self.transitions).enumerate()
        {
            let index = u32::try_from(index)
                .map_err(|_| NativeEpisodeHistoryError::new("transition index overflowed"))?;
            if current_episode != Some(decision.episode_id.as_str()) {
                if decision.step_index != 0 {
                    return Err(NativeEpisodeHistoryError::new(
                        "episode history does not reset at an episode boundary",
                    ));
                }
                current_episode = Some(decision.episode_id.as_str());
                completed.clear();
            } else if decision.step_index as usize != completed.len() {
                return Err(NativeEpisodeHistoryError::new(
                    "episode history step sequence is noncanonical",
                ));
            }

            let before = index
                .checked_mul(2)
                .ok_or_else(|| NativeEpisodeHistoryError::new("boundary index overflowed"))?;
            let after = before + 1;
            let pre = &self.boundaries[before as usize];
            let post = &self.boundaries[after as usize];
            let retained_from = completed.len().saturating_sub(self.history_depth);
            if decision.episode_id.is_empty()
                || decision.episode_id != transition.episode_id
                || decision.step_index != transition.step_index
                || decision.current_boundary_index != before
                || transition.before_boundary_index != before
                || transition.after_boundary_index != after
                || pre.episode_id != decision.episode_id
                || post.episode_id != decision.episode_id
                || pre.step_index != decision.step_index
                || post.step_index != decision.step_index
                || pre.phase != EpisodeHistoryPhase::PreInput
                || post.phase != EpisodeHistoryPhase::PostSimulation
                || decision.completed_transition_indices != completed[retained_from..]
                || decision
                    .completed_transition_indices
                    .iter()
                    .any(|prior| *prior >= index)
            {
                return Err(NativeEpisodeHistoryError::new(
                    "episode history contains a phase, episode, or future-history mismatch",
                ));
            }
            completed.push(index);
        }
        Ok(())
    }

    fn compute_identity(&self) -> Result<Digest, NativeEpisodeHistoryError> {
        let mut canonical = self.clone();
        canonical.view_sha256 = Digest::ZERO;
        let bytes = serde_json::to_vec(&canonical)
            .map_err(|error| NativeEpisodeHistoryError::new(error.to_string()))?;
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight.native-episode-history/v1\0");
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
        Ok(Digest(hasher.finalize().into()))
    }
}

#[allow(clippy::too_many_arguments)]
fn boundary(
    episode_id: &str,
    step_index: u32,
    source_observation_ordinal: u32,
    phase: EpisodeHistoryPhase,
    boundary_index: u64,
    simulation_tick: u64,
    tape_frame: u64,
    state_identity: [u8; 16],
    stage: &str,
    room: i8,
    layer: i8,
    point: i16,
) -> EpisodeHistoryBoundary {
    EpisodeHistoryBoundary {
        episode_id: episode_id.into(),
        step_index,
        phase,
        source_observation_ordinal,
        boundary_index,
        simulation_tick,
        tape_frame,
        state_identity_xxh3_128: hex(&state_identity),
        stage: stage.into(),
        room,
        layer,
        point,
    }
}

fn pad(value: NativeRawPad) -> EpisodeHistoryPad {
    EpisodeHistoryPad {
        buttons: value.buttons,
        stick_x: value.stick_x,
        stick_y: value.stick_y,
        substick_x: value.substick_x,
        substick_y: value.substick_y,
        trigger_left: value.trigger_left,
        trigger_right: value.trigger_right,
        analog_a: value.analog_a,
        analog_b: value.analog_b,
        connected: value.connected,
        error: value.error,
    }
}

fn terminal_reason(value: NativeTerminalReason) -> EpisodeHistoryTerminalReason {
    match value {
        NativeTerminalReason::None => EpisodeHistoryTerminalReason::None,
        NativeTerminalReason::GoalReached => EpisodeHistoryTerminalReason::GoalReached,
        NativeTerminalReason::TickBudgetExhausted => {
            EpisodeHistoryTerminalReason::TickBudgetExhausted
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[usize::from(byte >> 4)] as char);
        output.push(DIGITS[usize::from(byte & 0xf)] as char);
    }
    output
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeEpisodeHistoryError(String);

impl NativeEpisodeHistoryError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for NativeEpisodeHistoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for NativeEpisodeHistoryError {}

#[cfg(test)]
mod tests {
    use super::*;
    use dusklight_evidence::native_episode_shard::NativeObservationPhase;

    fn shard_with_steps(steps: usize, episodes: usize) -> NativeEpisodeShard {
        let mut shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v15.dseps"
        ))
        .unwrap();
        let template_episode = shard.episodes[0].clone();
        let template_step = template_episode.steps[0].clone();
        shard.episodes.clear();
        for episode_index in 0..episodes {
            let mut episode = template_episode.clone();
            episode.id = format!("episode-{episode_index}");
            episode.steps.clear();
            for step_index in 0..steps {
                let mut step = template_step.clone();
                step.pre_input.phase = NativeObservationPhase::PreInput;
                step.post_simulation.phase = NativeObservationPhase::PostSimulation;
                step.pre_input.boundary_index = step_index as u64;
                step.post_simulation.boundary_index = step_index as u64 + 1;
                step.pre_input.simulation_tick = step_index as u64;
                step.post_simulation.simulation_tick = step_index as u64 + 1;
                step.pre_input.tape_frame = step_index as u64;
                step.post_simulation.tape_frame = step_index as u64 + 1;
                step.pre_input.state_identity = [step_index as u8; 16];
                step.post_simulation.state_identity = [step_index as u8 + 1; 16];
                step.pre_input.terminal_reason = NativeTerminalReason::None;
                step.post_simulation.terminal_reason = if step_index + 1 == steps {
                    NativeTerminalReason::TickBudgetExhausted
                } else {
                    NativeTerminalReason::None
                };
                episode.steps.push(step);
            }
            shard.episodes.push(episode);
        }
        shard
    }

    #[test]
    fn builds_bounded_past_only_history_and_resets_each_episode() {
        let shard = shard_with_steps(5, 2);
        let view = NativeEpisodeHistoryView::build(&shard, 3).unwrap();
        assert_eq!(view.source_observation_count, 20);
        assert_eq!(view.decisions.len(), 10);
        assert_eq!(view.transitions.len(), 10);
        assert_eq!(
            view.decisions[0].completed_transition_indices,
            Vec::<u32>::new()
        );
        assert_eq!(
            view.decisions[3].completed_transition_indices,
            vec![0, 1, 2]
        );
        assert_eq!(
            view.decisions[4].completed_transition_indices,
            vec![1, 2, 3]
        );
        assert_eq!(
            view.decisions[5].completed_transition_indices,
            Vec::<u32>::new()
        );
        assert_eq!(
            view.decisions[9].completed_transition_indices,
            vec![6, 7, 8]
        );
        assert_eq!(
            view.current_boundary(4).unwrap().source_observation_ordinal,
            8
        );
        assert_eq!(
            view.resolved_history(9)
                .unwrap()
                .iter()
                .map(|transition| transition.step_index)
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
    }

    #[test]
    fn round_trips_canonically_and_preserves_exact_pads() {
        let shard = shard_with_steps(3, 1);
        let view = NativeEpisodeHistoryView::build(&shard, 2).unwrap();
        assert_eq!(
            view.transitions[0].chosen_pad.buttons,
            shard.episodes[0].steps[0].chosen_pad.buttons
        );
        assert_eq!(
            view.transitions[0].consumed_pad,
            pad(shard.episodes[0].steps[0].consumed_pad)
        );
        let bytes = view.canonical_bytes().unwrap();
        assert_eq!(
            NativeEpisodeHistoryView::decode_canonical(&bytes).unwrap(),
            view
        );
    }

    #[test]
    fn rejects_current_or_cross_episode_transition_as_history_even_when_resealed() {
        let shard = shard_with_steps(3, 2);
        let view = NativeEpisodeHistoryView::build(&shard, 3).unwrap();

        let mut future = view.clone();
        future.decisions[1].completed_transition_indices.push(1);
        future.view_sha256 = future.compute_identity().unwrap();
        assert!(
            future
                .validate()
                .unwrap_err()
                .to_string()
                .contains("future-history")
        );

        let mut crossed = view.clone();
        crossed.decisions[3].completed_transition_indices.push(2);
        crossed.view_sha256 = crossed.compute_identity().unwrap();
        assert!(
            crossed
                .validate()
                .unwrap_err()
                .to_string()
                .contains("future-history")
        );
    }

    #[test]
    fn invalid_depth_and_detached_source_fail_closed() {
        let shard = shard_with_steps(1, 1);
        assert!(NativeEpisodeHistoryView::build(&shard, 0).is_err());
        let mut detached = shard;
        detached.content_sha256 = Digest::ZERO;
        assert!(NativeEpisodeHistoryView::build(&detached, 1).is_err());
    }
}
