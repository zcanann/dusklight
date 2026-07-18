//! Advisory state-dependent action masks for search proposal ordering.
//!
//! These masks are deliberately absent from tape compilation, candidate
//! validation, native evaluation, corpus ingestion, and proof acceptance. They
//! may prioritize proposals, but can never declare a raw input invalid.

use serde::Serialize;
use std::error::Error;
use std::fmt;

pub const ACTION_GUIDANCE_SCHEMA_V1: &str = "dusklight-action-guidance/movement-v1";
pub const MOVEMENT_ACTION_COUNT_V2: u32 = 68;

const PLAYER_PRESENT_FEATURE: usize = 11;
const PLAYER_IS_LINK_FEATURE: usize = 12;
const EVENT_RUNNING_FEATURE: usize = 13;
const PAD_ERROR_FEATURE: usize = 34;
const REQUIRED_FEATURE_COUNT: usize = PAD_ERROR_FEATURE + 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GuidanceState {
    PadUnavailable,
    PlayerUnavailable,
    EventRunning,
    Gameplay,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AdvisoryActionMask {
    pub schema: &'static str,
    pub advisory: bool,
    pub state: GuidanceState,
    pub action_count: u32,
    pub recommended_actions: Vec<u32>,
}

impl AdvisoryActionMask {
    pub fn recommends(&self, action: u32) -> bool {
        self.recommended_actions.binary_search(&action).is_ok()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GuidanceError {
    FeatureCount {
        expected_at_least: usize,
        actual: usize,
    },
    NonFiniteFeature {
        index: usize,
    },
}

impl fmt::Display for GuidanceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FeatureCount {
                expected_at_least,
                actual,
            } => write!(
                formatter,
                "action guidance requires at least {expected_at_least} state features, got {actual}"
            ),
            Self::NonFiniteFeature { index } => {
                write!(formatter, "action guidance feature {index} is not finite")
            }
        }
    }
}

impl Error for GuidanceError {}

/// Builds the movement-v2 recommendation mask from movement-state/v1 slots.
///
/// The 68-action catalog is `button_mode * 17 + direction`, where direction
/// zero is neutral. Gameplay recommends the whole catalog. During events the
/// mask prefers unbuttoned motion plus neutral-stick button presses. Without a
/// live Link it prefers neutral-stick button states, and a pad error prefers a
/// completely neutral pad. These are proposal priors only.
pub fn movement_action_mask_v1(state: &[f32]) -> Result<AdvisoryActionMask, GuidanceError> {
    if state.len() < REQUIRED_FEATURE_COUNT {
        return Err(GuidanceError::FeatureCount {
            expected_at_least: REQUIRED_FEATURE_COUNT,
            actual: state.len(),
        });
    }
    for index in [
        PLAYER_PRESENT_FEATURE,
        PLAYER_IS_LINK_FEATURE,
        EVENT_RUNNING_FEATURE,
        PAD_ERROR_FEATURE,
    ] {
        if !state[index].is_finite() {
            return Err(GuidanceError::NonFiniteFeature { index });
        }
    }

    let state_class = if state[PAD_ERROR_FEATURE] != 0.0 {
        GuidanceState::PadUnavailable
    } else if state[PLAYER_PRESENT_FEATURE] < 0.5 || state[PLAYER_IS_LINK_FEATURE] < 0.5 {
        GuidanceState::PlayerUnavailable
    } else if state[EVENT_RUNNING_FEATURE] >= 0.5 {
        GuidanceState::EventRunning
    } else {
        GuidanceState::Gameplay
    };
    let recommended_actions = match state_class {
        GuidanceState::PadUnavailable => vec![0],
        GuidanceState::PlayerUnavailable => vec![0, 17, 34, 51],
        GuidanceState::EventRunning => {
            let mut actions = (0..17).collect::<Vec<_>>();
            actions.extend([17, 34, 51]);
            actions
        }
        GuidanceState::Gameplay => (0..MOVEMENT_ACTION_COUNT_V2).collect(),
    };
    Ok(AdvisoryActionMask {
        schema: ACTION_GUIDANCE_SCHEMA_V1,
        advisory: true,
        state: state_class,
        action_count: MOVEMENT_ACTION_COUNT_V2,
        recommended_actions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::offline_rl::canonical_movement_pad_v2;
    use crate::search::{Candidate, SegmentProfile};
    use crate::tape::{InputFrame, InputTape, RawPadState};

    fn state() -> Vec<f32> {
        let mut state = vec![0.0; REQUIRED_FEATURE_COUNT];
        state[PLAYER_PRESENT_FEATURE] = 1.0;
        state[PLAYER_IS_LINK_FEATURE] = 1.0;
        state
    }

    #[test]
    fn classifies_state_and_recommends_a_sorted_subset() {
        let gameplay = movement_action_mask_v1(&state()).unwrap();
        assert_eq!(gameplay.state, GuidanceState::Gameplay);
        assert_eq!(gameplay.recommended_actions.len(), 68);

        let mut event = state();
        event[EVENT_RUNNING_FEATURE] = 1.0;
        let event = movement_action_mask_v1(&event).unwrap();
        assert_eq!(event.state, GuidanceState::EventRunning);
        assert!(event.recommends(16));
        assert!(event.recommends(17));
        assert!(!event.recommends(18));
        assert!(!event.recommends(67));

        let mut unavailable = state();
        unavailable[PLAYER_PRESENT_FEATURE] = 0.0;
        assert_eq!(
            movement_action_mask_v1(&unavailable)
                .unwrap()
                .recommended_actions,
            [0, 17, 34, 51]
        );
        unavailable[PAD_ERROR_FEATURE] = -1.0;
        assert_eq!(
            movement_action_mask_v1(&unavailable)
                .unwrap()
                .recommended_actions,
            [0]
        );
    }

    #[test]
    fn masked_out_action_still_compiles_as_an_exact_candidate_proof_input() {
        let mut event = state();
        event[EVENT_RUNNING_FEATURE] = 1.0;
        let mask = movement_action_mask_v1(&event).unwrap();
        let action = 67;
        assert!(!mask.recommends(action));
        let pad = canonical_movement_pad_v2(action).unwrap();
        let disconnected = RawPadState {
            connected: false,
            error: -1,
            ..RawPadState::default()
        };
        let tape = InputTape {
            frames: vec![InputFrame {
                owned_ports: 1,
                pads: [pad, disconnected, disconnected, disconnected],
                ..InputFrame::default()
            }],
            ..InputTape::default()
        };
        let candidate =
            Candidate::from_absolute_tape(SegmentProfile::Fsp103ToFsp104, &tape).unwrap();
        assert_eq!(candidate.compile().unwrap(), tape);
    }
}
