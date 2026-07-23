//! Sealed population-collapse diagnostics for one realized policy generation.
//!
//! The report is derived only from authenticated native episode shards. It
//! deliberately reports exact diversity counts and explicit warning rules; it
//! does not infer progress from coordinates or replace terminal outcomes.

use crate::artifact::Digest;
use crate::native_replay_corpus::DemonstrationMode;
use dusklight_evidence::native_episode_shard::{NativeEpisodeShard, NativeRawPad};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

pub const NATIVE_POLICY_COLLAPSE_REPORT_SCHEMA_V1: &str =
    "dusklight-native-policy-collapse-report/v1";
const MINIMUM_POPULATION: u64 = 2;
const MINIMUM_DISTINCT: u64 = 2;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativePolicyCollapseWarning {
    InsufficientRollouts,
    SingleParentState,
    SingleConsumedAction,
    SingleActionTrajectory,
    SingleStateIdentity,
    SingleContactSignature,
    NoTerminalSuccess,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativePolicyCollapseReport {
    pub schema: String,
    pub report_sha256: Digest,
    pub generation: u16,
    pub demonstration_mode: DemonstrationMode,
    pub source_shards: Vec<Digest>,
    pub rollouts: u64,
    pub transitions: u64,
    pub unique_parent_states: u64,
    pub unique_consumed_actions: u64,
    pub unique_action_trajectories: u64,
    pub unique_state_identities: u64,
    pub contact_observations: u64,
    pub unique_contact_signatures: u64,
    pub successes: u64,
    pub failures: u64,
    pub unique_success_ticks: u64,
    pub minimum_population: u64,
    pub minimum_distinct: u64,
    pub collapse_detected: bool,
    pub warnings: Vec<NativePolicyCollapseWarning>,
}

impl NativePolicyCollapseReport {
    pub fn build(
        generation: u16,
        shards: &[NativeEpisodeShard],
    ) -> Result<Self, NativePolicyCollapseError> {
        Self::build_for_mode(
            generation,
            DemonstrationMode::BehaviorCloningWarmStart,
            shards,
        )
    }

    pub fn build_for_mode(
        generation: u16,
        demonstration_mode: DemonstrationMode,
        shards: &[NativeEpisodeShard],
    ) -> Result<Self, NativePolicyCollapseError> {
        if generation == 0 || shards.is_empty() {
            return Err(collapse_message(
                "policy-collapse report requires a generation and native shards",
            ));
        }
        let mut source_shards = shards
            .iter()
            .map(|shard| shard.content_sha256)
            .collect::<Vec<_>>();
        source_shards.sort_unstable();
        if source_shards.contains(&Digest::ZERO)
            || source_shards.windows(2).any(|pair| pair[0] == pair[1])
        {
            return Err(collapse_message(
                "policy-collapse source shards are zero or duplicated",
            ));
        }

        let mut parent_states = BTreeSet::new();
        let mut consumed_actions = BTreeSet::new();
        let mut action_trajectories = BTreeSet::new();
        let mut state_identities = BTreeSet::new();
        let mut contact_signatures = BTreeSet::new();
        let mut success_ticks = BTreeSet::new();
        let mut rollouts = 0_u64;
        let mut transitions = 0_u64;
        let mut contact_observations = 0_u64;
        let mut successes = 0_u64;
        for shard in shards {
            if shard.episodes.is_empty() {
                return Err(collapse_message(
                    "policy-collapse source shard has no episodes",
                ));
            }
            for episode in &shard.episodes {
                let first = episode.steps.first().ok_or_else(|| {
                    collapse_message("policy-collapse source episode has no transitions")
                })?;
                rollouts = rollouts
                    .checked_add(1)
                    .ok_or_else(|| collapse_message("policy-collapse rollout count overflowed"))?;
                parent_states.insert(first.pre_input.state_identity);
                successes = successes
                    .checked_add(u64::from(episode.success))
                    .ok_or_else(|| collapse_message("policy-collapse success count overflowed"))?;
                if let Some(tick) = episode.first_hit_tick {
                    success_ticks.insert(tick);
                }
                let mut trajectory = Vec::with_capacity(episode.steps.len() * 12);
                for step in &episode.steps {
                    transitions = transitions.checked_add(1).ok_or_else(|| {
                        collapse_message("policy-collapse transition count overflowed")
                    })?;
                    let action = pad_key(step.consumed_pad);
                    consumed_actions.insert(action);
                    encode_pad(action, &mut trajectory);
                    for observation in [&step.pre_input, &step.post_simulation] {
                        state_identities.insert(observation.state_identity);
                        contact_signatures.insert(observation.player_contacts);
                        contact_observations = contact_observations
                            .checked_add(u64::from(observation.player_contacts != 0))
                            .ok_or_else(|| {
                                collapse_message(
                                    "policy-collapse contact observation count overflowed",
                                )
                            })?;
                    }
                }
                action_trajectories.insert(sha256(&trajectory));
            }
        }
        let failures = rollouts
            .checked_sub(successes)
            .ok_or_else(|| collapse_message("policy-collapse outcomes are invalid"))?;
        let mut warnings = Vec::new();
        if rollouts < MINIMUM_POPULATION {
            warnings.push(NativePolicyCollapseWarning::InsufficientRollouts);
        } else {
            for (collapsed, warning) in [
                (
                    parent_states.len() < MINIMUM_DISTINCT as usize,
                    NativePolicyCollapseWarning::SingleParentState,
                ),
                (
                    consumed_actions.len() < MINIMUM_DISTINCT as usize,
                    NativePolicyCollapseWarning::SingleConsumedAction,
                ),
                (
                    action_trajectories.len() < MINIMUM_DISTINCT as usize,
                    NativePolicyCollapseWarning::SingleActionTrajectory,
                ),
                (
                    state_identities.len() < MINIMUM_DISTINCT as usize,
                    NativePolicyCollapseWarning::SingleStateIdentity,
                ),
                (
                    contact_signatures.len() < MINIMUM_DISTINCT as usize,
                    NativePolicyCollapseWarning::SingleContactSignature,
                ),
                (
                    successes == 0,
                    NativePolicyCollapseWarning::NoTerminalSuccess,
                ),
            ] {
                if collapsed {
                    warnings.push(warning);
                }
            }
        }
        let collapse_detected = rollouts >= MINIMUM_POPULATION && !warnings.is_empty();
        let mut report = Self {
            schema: NATIVE_POLICY_COLLAPSE_REPORT_SCHEMA_V1.into(),
            report_sha256: Digest::ZERO,
            generation,
            demonstration_mode,
            source_shards,
            rollouts,
            transitions,
            unique_parent_states: parent_states.len() as u64,
            unique_consumed_actions: consumed_actions.len() as u64,
            unique_action_trajectories: action_trajectories.len() as u64,
            unique_state_identities: state_identities.len() as u64,
            contact_observations,
            unique_contact_signatures: contact_signatures.len() as u64,
            successes,
            failures,
            unique_success_ticks: success_ticks.len() as u64,
            minimum_population: MINIMUM_POPULATION,
            minimum_distinct: MINIMUM_DISTINCT,
            collapse_detected,
            warnings,
        };
        report.report_sha256 = report.digest()?;
        report.validate()?;
        Ok(report)
    }

    pub fn validate(&self) -> Result<(), NativePolicyCollapseError> {
        if self.schema != NATIVE_POLICY_COLLAPSE_REPORT_SCHEMA_V1
            || self.generation == 0
            || self.source_shards.is_empty()
            || self.source_shards.contains(&Digest::ZERO)
            || !self.source_shards.windows(2).all(|pair| pair[0] < pair[1])
            || self.rollouts == 0
            || self.transitions < self.rollouts
            || self.successes.checked_add(self.failures) != Some(self.rollouts)
            || self.unique_parent_states == 0
            || self.unique_parent_states > self.rollouts
            || self.unique_consumed_actions == 0
            || self.unique_consumed_actions > self.transitions
            || self.unique_action_trajectories == 0
            || self.unique_action_trajectories > self.rollouts
            || self.unique_state_identities == 0
            || self.unique_contact_signatures == 0
            || self.unique_contact_signatures > 256
            || self.unique_success_ticks > self.successes
            || self.minimum_population != MINIMUM_POPULATION
            || self.minimum_distinct != MINIMUM_DISTINCT
            || !self.warnings.windows(2).all(|pair| pair[0] < pair[1])
            || self.collapse_detected
                != (self.rollouts >= MINIMUM_POPULATION && !self.warnings.is_empty())
            || self.report_sha256 == Digest::ZERO
            || self.report_sha256 != self.digest()?
        {
            return Err(collapse_message(
                "native policy-collapse report is invalid or detached",
            ));
        }
        if self.rollouts < MINIMUM_POPULATION
            && self.warnings != [NativePolicyCollapseWarning::InsufficientRollouts]
        {
            return Err(collapse_message(
                "insufficient policy-collapse population has invalid warnings",
            ));
        }
        Ok(())
    }

    pub fn validate_against(
        &self,
        generation: u16,
        shards: &[NativeEpisodeShard],
    ) -> Result<(), NativePolicyCollapseError> {
        if self != &Self::build(generation, shards)? {
            return Err(collapse_message(
                "policy-collapse report differs from its realized native shards",
            ));
        }
        Ok(())
    }

    pub fn validate_against_mode(
        &self,
        generation: u16,
        demonstration_mode: DemonstrationMode,
        shards: &[NativeEpisodeShard],
    ) -> Result<(), NativePolicyCollapseError> {
        if self != &Self::build_for_mode(generation, demonstration_mode, shards)? {
            return Err(collapse_message(
                "policy-collapse report differs from its demonstration treatment or realized native shards",
            ));
        }
        Ok(())
    }

    fn digest(&self) -> Result<Digest, NativePolicyCollapseError> {
        let mut canonical = self.clone();
        canonical.report_sha256 = Digest::ZERO;
        let bytes = serde_json::to_vec(&canonical).map_err(collapse_error)?;
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight.native-policy-collapse-report/v1\0");
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
        Ok(Digest(hasher.finalize().into()))
    }
}

type PadKey = (u16, i8, i8, i8, i8, u8, u8, u8, u8, bool, i8);

fn pad_key(pad: NativeRawPad) -> PadKey {
    (
        pad.buttons,
        pad.stick_x,
        pad.stick_y,
        pad.substick_x,
        pad.substick_y,
        pad.trigger_left,
        pad.trigger_right,
        pad.analog_a,
        pad.analog_b,
        pad.connected,
        pad.error,
    )
}

fn encode_pad(pad: PadKey, output: &mut Vec<u8>) {
    output.extend_from_slice(&pad.0.to_le_bytes());
    output.extend_from_slice(&[
        pad.1 as u8,
        pad.2 as u8,
        pad.3 as u8,
        pad.4 as u8,
        pad.5,
        pad.6,
        pad.7,
        pad.8,
        u8::from(pad.9),
        pad.10 as u8,
    ]);
}

fn sha256(bytes: &[u8]) -> Digest {
    Digest(Sha256::digest(bytes).into())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativePolicyCollapseError(String);

impl fmt::Display for NativePolicyCollapseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for NativePolicyCollapseError {}

fn collapse_message(message: impl Into<String>) -> NativePolicyCollapseError {
    NativePolicyCollapseError(message.into())
}

fn collapse_error(error: impl fmt::Display) -> NativePolicyCollapseError {
    collapse_message(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn fixture() -> NativeEpisodeShard {
        NativeEpisodeShard::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../../tests/fixtures/automation/native_episode_v27.dseps"),
        )
        .unwrap()
    }

    #[test]
    fn identical_rollouts_report_each_exact_collapse_dimension() {
        let first = fixture();
        let mut second = first.clone();
        second.content_sha256 = Digest([9; 32]);
        let report = NativePolicyCollapseReport::build(1, &[first, second]).unwrap();
        assert!(report.collapse_detected);
        assert_eq!(report.unique_parent_states, 1);
        assert_eq!(report.unique_action_trajectories, 1);
        assert!(
            report
                .warnings
                .contains(&NativePolicyCollapseWarning::SingleParentState)
        );
        assert!(
            report
                .warnings
                .contains(&NativePolicyCollapseWarning::SingleActionTrajectory)
        );
    }

    #[test]
    fn report_is_recomputed_from_shards_and_rejects_tampering() {
        let shard = fixture();
        let report = NativePolicyCollapseReport::build(3, std::slice::from_ref(&shard)).unwrap();
        report
            .validate_against(3, std::slice::from_ref(&shard))
            .unwrap();
        let mut tampered = report.clone();
        tampered.unique_state_identities += 1;
        assert!(tampered.validate_against(3, &[shard]).is_err());
    }

    #[test]
    fn comparison_identity_binds_the_demonstration_treatment() {
        let shard = fixture();
        let report = NativePolicyCollapseReport::build_for_mode(
            1,
            DemonstrationMode::ReplayOnly,
            std::slice::from_ref(&shard),
        )
        .unwrap();
        report
            .validate_against_mode(
                1,
                DemonstrationMode::ReplayOnly,
                std::slice::from_ref(&shard),
            )
            .unwrap();
        assert!(
            report
                .validate_against_mode(1, DemonstrationMode::BehaviorCloningWarmStart, &[shard],)
                .is_err()
        );
    }
}
