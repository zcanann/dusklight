//! Masked room/resource-load sets derived from authenticated native episodes.
//!
//! The view keeps the engine's complete fixed room table as one ordered set.
//! It never copies process IDs, handler pointers, archives, or a desired load
//! destination, and it preserves legacy channel missingness outside the set.

use crate::artifact::Digest;
use crate::native_actor_view::ActorViewObservationPhase;
use dusklight_evidence::native_episode_shard::{
    NativeChannelStatus, NativeEpisodeShard, NativeLearningObservation, NativeObservationPhase,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;

pub const NATIVE_ROOM_LOAD_VIEW_SCHEMA_V1: &str = "dusklight-native-room-load-view/v1";
const ROOM_COUNT: usize = 64;
const MEMORY_BLOCK_COUNT: i8 = 19;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomLoadSetStatus {
    NotSampled,
    Unavailable,
    Present,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomSceneSetStatus {
    Absent,
    Unavailable,
    Present,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeRoomLoadRow {
    pub room: u8,
    pub status_flags: u8,
    pub draw: bool,
    pub zone_count: i8,
    pub zone: i8,
    pub memory_block: i8,
    pub region: u8,
    pub scene_status: RoomSceneSetStatus,
    pub scene_phase: Option<i32>,
    pub scene_phase_active: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeRoomLoadFeatureSet {
    pub room_read: i8,
    pub stay_room: i8,
    pub old_stay_room: i8,
    pub next_stay_room: i8,
    pub no_change_room: bool,
    pub time_pass: bool,
    pub rooms: Vec<NativeRoomLoadRow>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeRoomLoadViewObservation {
    pub episode_id: String,
    pub step_index: u32,
    pub phase: ActorViewObservationPhase,
    pub boundary_index: u64,
    pub state_identity_xxh3_128: String,
    pub stage: String,
    pub room: i8,
    pub status: RoomLoadSetStatus,
    pub load: Option<NativeRoomLoadFeatureSet>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeEpisodeRoomLoadView {
    pub schema: String,
    pub native_shard_sha256: Digest,
    pub observation_schema: String,
    pub observations: Vec<NativeRoomLoadViewObservation>,
    pub view_sha256: Digest,
}

impl NativeEpisodeRoomLoadView {
    pub fn build(shard: &NativeEpisodeShard) -> Result<Self, NativeRoomLoadViewError> {
        if shard.content_sha256 == Digest::ZERO || shard.episodes.is_empty() {
            return Err(NativeRoomLoadViewError::new(
                "native room-load view requires an authenticated nonempty shard",
            ));
        }
        let mut observations = Vec::new();
        for episode in &shard.episodes {
            for (step_index, step) in episode.steps.iter().enumerate() {
                let step_index = u32::try_from(step_index)
                    .map_err(|_| NativeRoomLoadViewError::new("step index overflowed"))?;
                observations.push(materialize_observation(
                    &episode.id,
                    step_index,
                    &step.pre_input,
                )?);
                observations.push(materialize_observation(
                    &episode.id,
                    step_index,
                    &step.post_simulation,
                )?);
            }
        }
        let mut view = Self {
            schema: NATIVE_ROOM_LOAD_VIEW_SCHEMA_V1.into(),
            native_shard_sha256: shard.content_sha256,
            observation_schema: shard.metadata.observation_schema.clone(),
            observations,
            view_sha256: Digest::ZERO,
        };
        view.view_sha256 = view.compute_identity()?;
        view.validate()?;
        Ok(view)
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, NativeRoomLoadViewError> {
        self.validate()?;
        serde_json::to_vec(self).map_err(|error| NativeRoomLoadViewError::new(error.to_string()))
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, NativeRoomLoadViewError> {
        let view: Self = serde_json::from_slice(bytes)
            .map_err(|error| NativeRoomLoadViewError::new(error.to_string()))?;
        view.validate()?;
        if view.canonical_bytes()? != bytes {
            return Err(NativeRoomLoadViewError::new(
                "native room-load view bytes are not canonical",
            ));
        }
        Ok(view)
    }

    pub fn validate(&self) -> Result<(), NativeRoomLoadViewError> {
        if self.schema != NATIVE_ROOM_LOAD_VIEW_SCHEMA_V1
            || self.native_shard_sha256 == Digest::ZERO
            || self.observation_schema.is_empty()
            || self.observations.is_empty()
            || self.view_sha256 != self.compute_identity()?
        {
            return Err(NativeRoomLoadViewError::new(
                "native room-load view envelope or seal is invalid",
            ));
        }
        for observation in &self.observations {
            validate_observation(observation)?;
        }
        Ok(())
    }

    fn compute_identity(&self) -> Result<Digest, NativeRoomLoadViewError> {
        let mut canonical = self.clone();
        canonical.view_sha256 = Digest::ZERO;
        let bytes = serde_json::to_vec(&canonical)
            .map_err(|error| NativeRoomLoadViewError::new(error.to_string()))?;
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight.native-room-load-view/v1\0");
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
        Ok(Digest(hasher.finalize().into()))
    }
}

fn materialize_observation(
    episode_id: &str,
    step_index: u32,
    source: &NativeLearningObservation,
) -> Result<NativeRoomLoadViewObservation, NativeRoomLoadViewError> {
    let phase = match source.phase {
        NativeObservationPhase::PreInput => ActorViewObservationPhase::PreInput,
        NativeObservationPhase::PostSimulation => ActorViewObservationPhase::PostSimulation,
    };
    let (status, load) = match source.room_load_status {
        NativeChannelStatus::NotSampled => (RoomLoadSetStatus::NotSampled, None),
        NativeChannelStatus::Unavailable => (RoomLoadSetStatus::Unavailable, None),
        NativeChannelStatus::Present => {
            let source = source.room_load.as_ref().ok_or_else(|| {
                NativeRoomLoadViewError::new("present room-load channel has no payload")
            })?;
            let rooms = source
                .rooms
                .iter()
                .map(|room| {
                    let (scene_status, scene_phase, scene_phase_active) = match room.scene_status {
                        NativeChannelStatus::Absent => (RoomSceneSetStatus::Absent, None, None),
                        NativeChannelStatus::Unavailable => {
                            (RoomSceneSetStatus::Unavailable, None, None)
                        }
                        NativeChannelStatus::Present => (
                            RoomSceneSetStatus::Present,
                            Some(room.scene_phase),
                            Some(room.scene_phase_active),
                        ),
                        NativeChannelStatus::NotSampled => {
                            return Err(NativeRoomLoadViewError::new(
                                "room scene unexpectedly has not-sampled status",
                            ));
                        }
                    };
                    Ok(NativeRoomLoadRow {
                        room: room.room,
                        status_flags: room.status_flags,
                        draw: room.draw,
                        zone_count: room.zone_count,
                        zone: room.zone,
                        memory_block: room.memory_block,
                        region: room.region,
                        scene_status,
                        scene_phase,
                        scene_phase_active,
                    })
                })
                .collect::<Result<Vec<_>, NativeRoomLoadViewError>>()?;
            (
                RoomLoadSetStatus::Present,
                Some(NativeRoomLoadFeatureSet {
                    room_read: source.room_read,
                    stay_room: source.stay_room,
                    old_stay_room: source.old_stay_room,
                    next_stay_room: source.next_stay_room,
                    no_change_room: source.no_change_room,
                    time_pass: source.time_pass,
                    rooms,
                }),
            )
        }
        NativeChannelStatus::Absent => {
            return Err(NativeRoomLoadViewError::new(
                "room-load channel cannot be semantically absent",
            ));
        }
    };
    Ok(NativeRoomLoadViewObservation {
        episode_id: episode_id.into(),
        step_index,
        phase,
        boundary_index: source.boundary_index,
        state_identity_xxh3_128: lower_hex(&source.state_identity),
        stage: source.stage.clone(),
        room: source.room,
        status,
        load,
    })
}

fn validate_observation(
    observation: &NativeRoomLoadViewObservation,
) -> Result<(), NativeRoomLoadViewError> {
    if observation.episode_id.is_empty()
        || observation.stage.is_empty()
        || !is_lower_hex(&observation.state_identity_xxh3_128, 32)
        || (observation.status == RoomLoadSetStatus::Present) != observation.load.is_some()
    {
        return Err(NativeRoomLoadViewError::new(
            "native room-load observation is invalid",
        ));
    }
    let Some(load) = &observation.load else {
        return Ok(());
    };
    let valid_room = |room: i8| (-1..64).contains(&room);
    if !valid_room(load.room_read)
        || !valid_room(load.stay_room)
        || !valid_room(load.old_stay_room)
        || !valid_room(load.next_stay_room)
        || load.rooms.len() != ROOM_COUNT
    {
        return Err(NativeRoomLoadViewError::new(
            "native room-load global state is invalid",
        ));
    }
    for (index, room) in load.rooms.iter().enumerate() {
        let scene_present = room.scene_status == RoomSceneSetStatus::Present;
        if usize::from(room.room) != index
            || room.zone < -1
            || !(-1..MEMORY_BLOCK_COUNT).contains(&room.memory_block)
            || scene_present != room.scene_phase.is_some()
            || scene_present != room.scene_phase_active.is_some()
            || room
                .scene_phase
                .is_some_and(|phase| !(0..=4).contains(&phase))
            || (scene_present && room.status_flags == 0)
        {
            return Err(NativeRoomLoadViewError::new(
                "native room-load row is invalid",
            ));
        }
    }
    Ok(())
}

fn lower_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(DIGITS[usize::from(byte >> 4)] as char);
        encoded.push(DIGITS[usize::from(byte & 0x0f)] as char);
    }
    encoded
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeRoomLoadViewError(String);

impl NativeRoomLoadViewError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for NativeRoomLoadViewError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for NativeRoomLoadViewError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn shard(bytes: &[u8]) -> NativeEpisodeShard {
        let mut shard = NativeEpisodeShard::decode(bytes).unwrap();
        shard.episodes.truncate(1);
        shard.episodes[0].steps.truncate(1);
        shard
    }

    #[test]
    fn v24_becomes_a_complete_masked_room_set() {
        let shard = shard(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v24.dseps"
        ));
        let view = NativeEpisodeRoomLoadView::build(&shard).unwrap();
        assert_eq!(view.observations.len(), 2);
        let observation = &view.observations[0];
        assert_eq!(observation.status, RoomLoadSetStatus::Present);
        let load = observation.load.as_ref().unwrap();
        assert_eq!(
            (load.room_read, load.stay_room, load.next_stay_room),
            (1, 0, 1)
        );
        assert_eq!(load.rooms.len(), ROOM_COUNT);
        assert_eq!(load.rooms[0].status_flags, 0x11);
        assert_eq!(load.rooms[0].scene_status, RoomSceneSetStatus::Present);
        assert_eq!(load.rooms[0].scene_phase, Some(3));
        assert_eq!(load.rooms[0].scene_phase_active, Some(true));
        assert_eq!(load.rooms[1].scene_status, RoomSceneSetStatus::Absent);
        assert_eq!(load.rooms[1].scene_phase, None);
        assert_eq!(load.rooms[63].room, 63);

        let bytes = view.canonical_bytes().unwrap();
        assert_eq!(
            NativeEpisodeRoomLoadView::decode_canonical(&bytes).unwrap(),
            view
        );
    }

    #[test]
    fn legacy_missingness_does_not_create_room_rows() {
        let shard = shard(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v23.dseps"
        ));
        let view = NativeEpisodeRoomLoadView::build(&shard).unwrap();
        assert!(view.observations.iter().all(|observation| {
            observation.status == RoomLoadSetStatus::NotSampled && observation.load.is_none()
        }));
    }

    #[test]
    fn resealed_row_order_and_phase_tampering_fail_closed() {
        let shard = shard(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v24.dseps"
        ));
        let view = NativeEpisodeRoomLoadView::build(&shard).unwrap();

        let mut order = view.clone();
        order.observations[0].load.as_mut().unwrap().rooms[0].room = 1;
        order.view_sha256 = order.compute_identity().unwrap();
        assert!(order.validate().is_err());

        let mut phase = view;
        phase.observations[0].load.as_mut().unwrap().rooms[0].scene_phase = Some(5);
        phase.view_sha256 = phase.compute_identity().unwrap();
        assert!(phase.validate().is_err());
    }
}
