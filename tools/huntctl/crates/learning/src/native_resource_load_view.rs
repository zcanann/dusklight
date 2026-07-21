//! Masked resource-control sets derived from authenticated native episodes.
//!
//! Each present observation retains every occupied object/stage archive slot in
//! canonical engine-table order. Archive names remain categorical strings for a
//! later set/token encoder; pointers, addresses, resource bytes and a desired
//! archive are intentionally absent.

use crate::artifact::Digest;
use crate::native_actor_view::ActorViewObservationPhase;
use dusklight_evidence::native_episode_shard::{
    NativeChannelStatus, NativeEpisodeShard, NativeLearningObservation, NativeObservationPhase,
    NativeResourceArchiveKind, NativeResourceLoadOutcome,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;

pub const NATIVE_RESOURCE_LOAD_VIEW_SCHEMA_V1: &str = "dusklight-native-resource-load-view/v1";
const OBJECT_CAPACITY: u16 = 128;
const STAGE_CAPACITY: u16 = 64;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceLoadSetStatus {
    NotSampled,
    Unavailable,
    Present,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceArchiveKind {
    Object,
    Stage,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceLoadOutcome {
    Mounting,
    Ready,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeResourceLoadRow {
    pub kind: ResourceArchiveKind,
    pub slot: u8,
    pub archive_name: String,
    pub reference_count: u16,
    pub outcome: ResourceLoadOutcome,
    pub mount_command_present: bool,
    pub archive_present: bool,
    pub data_heap_present: bool,
    pub resource_table_present: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeResourceLoadFeatureSet {
    pub object_capacity: u16,
    pub stage_capacity: u16,
    pub object_count: u16,
    pub stage_count: u16,
    pub archives: Vec<NativeResourceLoadRow>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeResourceLoadViewObservation {
    pub episode_id: String,
    pub step_index: u32,
    pub phase: ActorViewObservationPhase,
    pub boundary_index: u64,
    pub state_identity_xxh3_128: String,
    pub stage: String,
    pub room: i8,
    pub status: ResourceLoadSetStatus,
    pub loads: Option<NativeResourceLoadFeatureSet>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeEpisodeResourceLoadView {
    pub schema: String,
    pub native_shard_sha256: Digest,
    pub observation_schema: String,
    pub observations: Vec<NativeResourceLoadViewObservation>,
    pub view_sha256: Digest,
}

impl NativeEpisodeResourceLoadView {
    pub fn build(shard: &NativeEpisodeShard) -> Result<Self, NativeResourceLoadViewError> {
        if shard.content_sha256 == Digest::ZERO || shard.episodes.is_empty() {
            return Err(NativeResourceLoadViewError::new(
                "native resource-load view requires an authenticated nonempty shard",
            ));
        }
        let mut observations = Vec::new();
        for episode in &shard.episodes {
            for (step_index, step) in episode.steps.iter().enumerate() {
                let step_index = u32::try_from(step_index)
                    .map_err(|_| NativeResourceLoadViewError::new("step index overflowed"))?;
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
            schema: NATIVE_RESOURCE_LOAD_VIEW_SCHEMA_V1.into(),
            native_shard_sha256: shard.content_sha256,
            observation_schema: shard.metadata.observation_schema.clone(),
            observations,
            view_sha256: Digest::ZERO,
        };
        view.view_sha256 = view.compute_identity()?;
        view.validate()?;
        view.verify_source_shard(shard)?;
        Ok(view)
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, NativeResourceLoadViewError> {
        self.validate()?;
        serde_json::to_vec(self)
            .map_err(|error| NativeResourceLoadViewError::new(error.to_string()))
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, NativeResourceLoadViewError> {
        let view: Self = serde_json::from_slice(bytes)
            .map_err(|error| NativeResourceLoadViewError::new(error.to_string()))?;
        view.validate()?;
        if view.canonical_bytes()? != bytes {
            return Err(NativeResourceLoadViewError::new(
                "native resource-load view bytes are not canonical",
            ));
        }
        Ok(view)
    }

    pub fn validate(&self) -> Result<(), NativeResourceLoadViewError> {
        if self.schema != NATIVE_RESOURCE_LOAD_VIEW_SCHEMA_V1
            || self.native_shard_sha256 == Digest::ZERO
            || self.observation_schema.is_empty()
            || self.observations.is_empty()
            || self.view_sha256 != self.compute_identity()?
        {
            return Err(NativeResourceLoadViewError::new(
                "native resource-load view envelope or seal is invalid",
            ));
        }
        for observation in &self.observations {
            validate_observation(observation)?;
        }
        Ok(())
    }

    pub fn verify_source_shard(
        &self,
        shard: &NativeEpisodeShard,
    ) -> Result<(), NativeResourceLoadViewError> {
        self.validate()?;
        let expected_count = shard
            .episodes
            .iter()
            .try_fold(0_usize, |total, episode| {
                total.checked_add(episode.steps.len().checked_mul(2)?)
            })
            .ok_or_else(|| NativeResourceLoadViewError::new("source size overflowed"))?;
        if self.native_shard_sha256 != shard.content_sha256
            || self.observation_schema != shard.metadata.observation_schema
            || self.observations.len() != expected_count
        {
            return Err(NativeResourceLoadViewError::new(
                "native resource-load view is detached from its source shard",
            ));
        }
        let mut retained = self.observations.iter();
        for episode in &shard.episodes {
            for (step_index, step) in episode.steps.iter().enumerate() {
                let step_index = u32::try_from(step_index).unwrap_or(u32::MAX);
                for source in [&step.pre_input, &step.post_simulation] {
                    let expected = materialize_observation(&episode.id, step_index, source)?;
                    if retained.next() != Some(&expected) {
                        return Err(NativeResourceLoadViewError::new(
                            "native resource-load rows disagree with the source shard",
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    fn compute_identity(&self) -> Result<Digest, NativeResourceLoadViewError> {
        let mut canonical = self.clone();
        canonical.view_sha256 = Digest::ZERO;
        let bytes = serde_json::to_vec(&canonical)
            .map_err(|error| NativeResourceLoadViewError::new(error.to_string()))?;
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight.native-resource-load-view/v1\0");
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
        Ok(Digest(hasher.finalize().into()))
    }
}

fn materialize_observation(
    episode_id: &str,
    step_index: u32,
    source: &NativeLearningObservation,
) -> Result<NativeResourceLoadViewObservation, NativeResourceLoadViewError> {
    let phase = match source.phase {
        NativeObservationPhase::PreInput => ActorViewObservationPhase::PreInput,
        NativeObservationPhase::PostSimulation => ActorViewObservationPhase::PostSimulation,
    };
    let (status, loads) = match source.resource_load_status {
        NativeChannelStatus::NotSampled => (ResourceLoadSetStatus::NotSampled, None),
        NativeChannelStatus::Unavailable => (ResourceLoadSetStatus::Unavailable, None),
        NativeChannelStatus::Present => {
            let source = source.resource_loads.as_ref().ok_or_else(|| {
                NativeResourceLoadViewError::new("present resource-load channel has no payload")
            })?;
            let archives = source
                .entries
                .iter()
                .map(|entry| NativeResourceLoadRow {
                    kind: match entry.kind {
                        NativeResourceArchiveKind::Object => ResourceArchiveKind::Object,
                        NativeResourceArchiveKind::Stage => ResourceArchiveKind::Stage,
                    },
                    slot: entry.slot,
                    archive_name: entry.archive_name.clone(),
                    reference_count: entry.reference_count,
                    outcome: match entry.outcome {
                        NativeResourceLoadOutcome::Mounting => ResourceLoadOutcome::Mounting,
                        NativeResourceLoadOutcome::Ready => ResourceLoadOutcome::Ready,
                        NativeResourceLoadOutcome::Failed => ResourceLoadOutcome::Failed,
                    },
                    mount_command_present: entry.mount_command_present,
                    archive_present: entry.archive_present,
                    data_heap_present: entry.data_heap_present,
                    resource_table_present: entry.resource_table_present,
                })
                .collect();
            (
                ResourceLoadSetStatus::Present,
                Some(NativeResourceLoadFeatureSet {
                    object_capacity: source.object_capacity,
                    stage_capacity: source.stage_capacity,
                    object_count: source.object_count,
                    stage_count: source.stage_count,
                    archives,
                }),
            )
        }
        NativeChannelStatus::Absent => {
            return Err(NativeResourceLoadViewError::new(
                "resource-load channel cannot be semantically absent",
            ));
        }
    };
    Ok(NativeResourceLoadViewObservation {
        episode_id: episode_id.into(),
        step_index,
        phase,
        boundary_index: source.boundary_index,
        state_identity_xxh3_128: lower_hex(&source.state_identity),
        stage: source.stage.clone(),
        room: source.room,
        status,
        loads,
    })
}

fn validate_observation(
    observation: &NativeResourceLoadViewObservation,
) -> Result<(), NativeResourceLoadViewError> {
    if observation.episode_id.is_empty()
        || observation.stage.is_empty()
        || !is_lower_hex(&observation.state_identity_xxh3_128, 32)
        || (observation.status == ResourceLoadSetStatus::Present) != observation.loads.is_some()
    {
        return Err(NativeResourceLoadViewError::new(
            "native resource-load observation is invalid",
        ));
    }
    let Some(loads) = &observation.loads else {
        return Ok(());
    };
    let total_count = loads.object_count.checked_add(loads.stage_count);
    if loads.object_capacity != OBJECT_CAPACITY
        || loads.stage_capacity != STAGE_CAPACITY
        || loads.object_count > OBJECT_CAPACITY
        || loads.stage_count > STAGE_CAPACITY
        || total_count.map(usize::from) != Some(loads.archives.len())
    {
        return Err(NativeResourceLoadViewError::new(
            "native resource-load set counts are invalid",
        ));
    }
    let mut previous_object = None;
    let mut previous_stage = None;
    for (index, archive) in loads.archives.iter().enumerate() {
        let ordered = match archive.kind {
            ResourceArchiveKind::Object => {
                index < usize::from(loads.object_count)
                    && u16::from(archive.slot) < OBJECT_CAPACITY
                    && previous_object.is_none_or(|previous| archive.slot > previous)
            }
            ResourceArchiveKind::Stage => {
                index >= usize::from(loads.object_count)
                    && u16::from(archive.slot) < STAGE_CAPACITY
                    && previous_stage.is_none_or(|previous| archive.slot > previous)
            }
        };
        let structural_outcome = if archive.mount_command_present
            && !archive.archive_present
            && !archive.data_heap_present
            && !archive.resource_table_present
        {
            Some(ResourceLoadOutcome::Mounting)
        } else if !archive.mount_command_present
            && archive.archive_present
            && archive.resource_table_present
        {
            Some(ResourceLoadOutcome::Ready)
        } else if !archive.mount_command_present
            && !archive.resource_table_present
            && (!archive.data_heap_present || archive.archive_present)
        {
            Some(ResourceLoadOutcome::Failed)
        } else {
            None
        };
        if !ordered
            || archive.reference_count == 0
            || archive.archive_name.is_empty()
            || archive.archive_name.len() > 11
            || !archive
                .archive_name
                .bytes()
                .all(|byte| byte.is_ascii_graphic())
            || structural_outcome != Some(archive.outcome)
        {
            return Err(NativeResourceLoadViewError::new(
                "native resource-load row is invalid",
            ));
        }
        match archive.kind {
            ResourceArchiveKind::Object => previous_object = Some(archive.slot),
            ResourceArchiveKind::Stage => previous_stage = Some(archive.slot),
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
pub struct NativeResourceLoadViewError(String);

impl NativeResourceLoadViewError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for NativeResourceLoadViewError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for NativeResourceLoadViewError {}

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
    fn v26_becomes_a_complete_masked_resource_set() {
        let shard = shard(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v26.dseps"
        ));
        let view = NativeEpisodeResourceLoadView::build(&shard).unwrap();
        assert_eq!(view.observations.len(), 2);
        let observation = &view.observations[0];
        assert_eq!(observation.status, ResourceLoadSetStatus::Present);
        let loads = observation.loads.as_ref().unwrap();
        assert_eq!((loads.object_capacity, loads.stage_capacity), (128, 64));
        assert_eq!((loads.object_count, loads.stage_count), (2, 1));
        assert_eq!(loads.archives.len(), 3);
        assert_eq!(loads.archives[0].archive_name, "ObjA");
        assert_eq!(loads.archives[0].outcome, ResourceLoadOutcome::Mounting);
        assert_eq!(loads.archives[1].archive_name, "Always");
        assert_eq!(loads.archives[1].outcome, ResourceLoadOutcome::Ready);
        assert_eq!(loads.archives[2].kind, ResourceArchiveKind::Stage);
        assert_eq!(loads.archives[2].outcome, ResourceLoadOutcome::Failed);

        let bytes = view.canonical_bytes().unwrap();
        assert_eq!(
            NativeEpisodeResourceLoadView::decode_canonical(&bytes).unwrap(),
            view
        );
        view.verify_source_shard(&shard).unwrap();
    }

    #[test]
    fn legacy_missingness_does_not_create_resource_rows() {
        let shard = shard(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v25.dseps"
        ));
        let view = NativeEpisodeResourceLoadView::build(&shard).unwrap();
        assert!(view.observations.iter().all(|observation| {
            observation.status == ResourceLoadSetStatus::NotSampled && observation.loads.is_none()
        }));
    }

    #[test]
    fn unavailable_channel_does_not_fabricate_resource_rows() {
        let mut shard = shard(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v26.dseps"
        ));
        let step = &mut shard.episodes[0].steps[0];
        for observation in [&mut step.pre_input, &mut step.post_simulation] {
            observation.resource_load_status = NativeChannelStatus::Unavailable;
            observation.resource_loads = None;
        }
        let view = NativeEpisodeResourceLoadView::build(&shard).unwrap();
        assert!(view.observations.iter().all(|observation| {
            observation.status == ResourceLoadSetStatus::Unavailable && observation.loads.is_none()
        }));
    }

    #[test]
    fn resealed_order_outcome_and_source_detachment_fail_closed() {
        let shard = shard(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v26.dseps"
        ));
        let view = NativeEpisodeResourceLoadView::build(&shard).unwrap();

        let mut order = view.clone();
        order.observations[0].loads.as_mut().unwrap().archives[1].slot = 2;
        order.view_sha256 = order.compute_identity().unwrap();
        assert!(order.validate().is_err());

        let mut outcome = view.clone();
        outcome.observations[0].loads.as_mut().unwrap().archives[0].outcome =
            ResourceLoadOutcome::Ready;
        outcome.view_sha256 = outcome.compute_identity().unwrap();
        assert!(outcome.validate().is_err());

        let mut shortened = view;
        let loads = shortened.observations[0].loads.as_mut().unwrap();
        loads.archives.remove(0);
        loads.object_count -= 1;
        shortened.view_sha256 = shortened.compute_identity().unwrap();
        shortened.validate().unwrap();
        assert!(shortened.verify_source_shard(&shard).is_err());
    }
}
