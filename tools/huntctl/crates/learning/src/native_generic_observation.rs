//! Stable generic observation authority for native discovery campaigns.
//!
//! The wire observation remains the source of truth. This module proves that a
//! shard supplies the complete, phase-correct channel family required by a
//! generic learner and binds bounded past-only camera/PAD history without
//! copying or selecting a route-specific subset.

use crate::artifact::Digest;
use crate::native_episode_history::NativeEpisodeHistoryView;
use dusklight_evidence::native_episode_shard::{
    LEARNING_OBSERVATION_SCHEMA_V27, LEARNING_OBSERVATION_SCHEMA_V28, NativeActorSelectionRule,
    NativeChannelStatus, NativeEpisodeShard, NativeLearningObservation, RAW_PAD_ACTION_SCHEMA_V2,
};
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;

pub const NATIVE_GENERIC_OBSERVATION_CONTRACT_SCHEMA_V1: &str =
    "dusklight-native-generic-observation-contract/v1";
pub const NATIVE_GENERIC_OBSERVATION_HISTORY_DEPTH: usize = 4;

pub fn canonical_native_generic_observation_contract() -> String {
    include_str!(
        "../../../../../tests/fixtures/automation/native_generic_observation_contract_v1.schema"
    )
    .replace("\r\n", "\n")
}

pub fn native_generic_observation_contract_sha256() -> Digest {
    Digest(Sha256::digest(canonical_native_generic_observation_contract().as_bytes()).into())
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeGenericObservationReport {
    pub schema: &'static str,
    pub contract_sha256: Digest,
    pub observation_schema: String,
    pub action_schema: String,
    pub history_depth: usize,
    pub episodes: usize,
    pub observations: usize,
    pub actor_observations: usize,
    pub trigger_volume_observations: usize,
    pub explicit_missingness: bool,
    pub complete_actor_sets: bool,
    pub past_only_camera_pad_history: bool,
}

pub fn validate_native_generic_observation_shard(
    shard: &NativeEpisodeShard,
) -> Result<NativeGenericObservationReport, NativeGenericObservationError> {
    if !matches!(
        shard.metadata.observation_schema.as_str(),
        LEARNING_OBSERVATION_SCHEMA_V27 | LEARNING_OBSERVATION_SCHEMA_V28
    ) || shard.metadata.action_schema != RAW_PAD_ACTION_SCHEMA_V2
        || shard.episodes.is_empty()
    {
        return Err(observation_error(
            "generic observation shard lacks the current observation/action authority",
        ));
    }
    let history = NativeEpisodeHistoryView::build(shard, NATIVE_GENERIC_OBSERVATION_HISTORY_DEPTH)
        .map_err(observation_error)?;
    let mut observations = 0_usize;
    let mut actor_observations = 0_usize;
    let mut trigger_volume_observations = 0_usize;
    for observation in shard.episodes.iter().flat_map(|episode| {
        episode
            .steps
            .iter()
            .flat_map(|step| [&step.pre_input, &step.post_simulation])
    }) {
        validate_observation(observation)?;
        observations += 1;
        actor_observations = actor_observations
            .checked_add(observation.actors.len())
            .ok_or_else(|| observation_error("generic actor observation count overflowed"))?;
        trigger_volume_observations = trigger_volume_observations
            .checked_add(
                observation
                    .actors
                    .iter()
                    .filter(|actor| actor.trigger_volume.is_some())
                    .count(),
            )
            .ok_or_else(|| observation_error("generic trigger observation count overflowed"))?;
    }
    if observations == 0 || history.decisions.len() != observations / 2 {
        return Err(observation_error(
            "generic observation history cardinality is detached from its episodes",
        ));
    }
    Ok(NativeGenericObservationReport {
        schema: NATIVE_GENERIC_OBSERVATION_CONTRACT_SCHEMA_V1,
        contract_sha256: native_generic_observation_contract_sha256(),
        observation_schema: shard.metadata.observation_schema.clone(),
        action_schema: shard.metadata.action_schema.clone(),
        history_depth: NATIVE_GENERIC_OBSERVATION_HISTORY_DEPTH,
        episodes: shard.episodes.len(),
        observations,
        actor_observations,
        trigger_volume_observations,
        explicit_missingness: true,
        complete_actor_sets: true,
        past_only_camera_pad_history: true,
    })
}

fn validate_observation(
    observation: &NativeLearningObservation,
) -> Result<(), NativeGenericObservationError> {
    if observation.actor_selection != NativeActorSelectionRule::Complete
        || observation.actors_truncated
        || observation.actor_observed_count as usize != observation.actors.len()
    {
        return Err(observation_error(
            "generic observation does not contain the complete actor set",
        ));
    }
    let required = [
        observation.camera_status,
        observation.player_action_status,
        observation.player_background_collision_status,
        observation.player_collision_surfaces_status,
        observation.scene_exit_status,
        observation.dynamic_colliders_status,
        observation.player_resources_status,
        observation.player_relationships_status,
        observation.player_collision_solver_status,
        observation.process_lifecycle_status,
        observation.event_transition_status,
        observation.room_load_status,
        observation.warp_session_status,
        observation.resource_load_status,
    ];
    if required.contains(&NativeChannelStatus::NotSampled) {
        return Err(observation_error(
            "generic observation contains an unsampled required channel",
        ));
    }
    if observation
        .scene_exit
        .as_ref()
        .is_some_and(|exit| !exit.signed_distance_to_volume.is_finite())
        || observation.actors.iter().any(|actor| {
            actor.trigger_volume.as_ref().is_some_and(|trigger| {
                trigger
                    .center
                    .iter()
                    .chain(&trigger.half_extent)
                    .any(|value| !value.is_finite())
            })
        })
    {
        return Err(observation_error(
            "generic target-trigger relation contains non-finite geometry",
        ));
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeGenericObservationError(String);

fn observation_error(error: impl fmt::Display) -> NativeGenericObservationError {
    NativeGenericObservationError(error.to_string())
}

impl fmt::Display for NativeGenericObservationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for NativeGenericObservationError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_native_shard_satisfies_the_generic_contract() {
        let shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v27.dseps"
        ))
        .unwrap();
        let report = validate_native_generic_observation_shard(&shard).unwrap();
        assert_eq!(report.observations, report.episodes * 2);
        assert!(report.complete_actor_sets);
        assert!(report.explicit_missingness);
        assert!(report.past_only_camera_pad_history);
    }

    #[test]
    fn legacy_or_truncated_actor_observations_fail_closed() {
        let legacy = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v26.dseps"
        ))
        .unwrap();
        assert!(validate_native_generic_observation_shard(&legacy).is_err());

        let mut current = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v27.dseps"
        ))
        .unwrap();
        current.episodes[0].steps[0].pre_input.actors_truncated = true;
        assert!(validate_native_generic_observation_shard(&current).is_err());
    }
}
