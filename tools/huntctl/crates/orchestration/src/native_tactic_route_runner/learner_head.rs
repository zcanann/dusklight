use super::{
    Digest, NativeTacticRouteRunError, TacticReplayControlPlane, route_error, route_message,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;

const LEARNER_HEAD_SCHEMA_V1: &str = "dusklight-campaign-learner-head/v1";
const JOURNAL_MAGIC: &[u8; 8] = b"DSKLHJ01";
const JOURNAL_VERSION: u16 = 1;
const JOURNAL_HEADER_BYTES: usize = 8 + 2 + 2 + 32;
const RECORD_HEADER_BYTES: usize = 4 + 32;
const MAXIMUM_RECORD_BYTES: usize = 64 * 1024;
const MAXIMUM_RECORDS: usize = 10_000_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct CampaignLearnerHead {
    pub(super) learner_snapshot_sha256: Digest,
    pub(super) replay_revision: u64,
    pub(super) model_revision: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredCampaignLearnerHead {
    schema: String,
    sequence: u64,
    replay_identity_sha256: Digest,
    learner_snapshot_sha256: Digest,
    replay_revision: u64,
    model_revision: u64,
    parent_head_sha256: Digest,
    head_sha256: Digest,
}

impl StoredCampaignLearnerHead {
    fn expected_head_sha256(&self) -> Result<Digest, NativeTacticRouteRunError> {
        let raw = serde_cbor::to_vec(&(
            &self.schema,
            self.sequence,
            self.replay_identity_sha256,
            self.learner_snapshot_sha256,
            self.replay_revision,
            self.model_revision,
            self.parent_head_sha256,
        ))
        .map_err(route_error)?;
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight.campaign-learner-head/v1\0");
        hasher.update(raw);
        Ok(Digest(hasher.finalize().into()))
    }

    fn public(&self) -> CampaignLearnerHead {
        CampaignLearnerHead {
            learner_snapshot_sha256: self.learner_snapshot_sha256,
            replay_revision: self.replay_revision,
            model_revision: self.model_revision,
        }
    }
}

/// Durable publication order for the single campaign learner authority.
///
/// Replay admissions bind the policy used by each worker, but a stale worker
/// can legitimately publish an older policy identity. This separate journal
/// preserves the authority's actual fitted-policy head across that case. A
/// partial final record is recoverable; corruption of a complete record is
/// fail-closed.
pub(super) struct CampaignLearnerHeadJournal {
    file: File,
    replay_identity_sha256: Digest,
    records: Vec<StoredCampaignLearnerHead>,
}

impl CampaignLearnerHeadJournal {
    pub(super) fn open_or_create(
        replay: &TacticReplayControlPlane,
    ) -> Result<Self, NativeTacticRouteRunError> {
        let replay_identity_sha256 = replay.identity().content_sha256().map_err(route_error)?;
        let path = journal_path(replay);
        if !path.exists() {
            let mut file = OpenOptions::new()
                .create_new(true)
                .read(true)
                .write(true)
                .open(&path)
                .map_err(route_error)?;
            write_header(&mut file, replay_identity_sha256)?;
            return Ok(Self {
                file,
                replay_identity_sha256,
                records: Vec::new(),
            });
        }

        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .map_err(route_error)?;
        let file_len = file.metadata().map_err(route_error)?.len();
        if file_len < JOURNAL_HEADER_BYTES as u64 {
            return Err(route_message(
                "campaign learner-head journal header is truncated",
            ));
        }
        let mut header = [0_u8; JOURNAL_HEADER_BYTES];
        file.read_exact(&mut header).map_err(route_error)?;
        let version = u16::from_le_bytes(header[8..10].try_into().expect("fixed slice"));
        let flags = u16::from_le_bytes(header[10..12].try_into().expect("fixed slice"));
        let stored_identity = Digest(header[12..44].try_into().expect("fixed slice"));
        if &header[..8] != JOURNAL_MAGIC
            || version != JOURNAL_VERSION
            || flags != 0
            || stored_identity != replay_identity_sha256
        {
            return Err(route_message(
                "campaign learner-head journal belongs to another replay authority",
            ));
        }

        let mut records: Vec<StoredCampaignLearnerHead> = Vec::new();
        let mut valid_len = JOURNAL_HEADER_BYTES as u64;
        let mut parent_head_sha256 = Digest::ZERO;
        loop {
            if records.len() >= MAXIMUM_RECORDS {
                return Err(route_message(
                    "campaign learner-head journal exceeds its record bound",
                ));
            }
            let remaining = file_len.saturating_sub(valid_len);
            if remaining == 0 {
                break;
            }
            if remaining < RECORD_HEADER_BYTES as u64 {
                break;
            }
            let mut record_header = [0_u8; RECORD_HEADER_BYTES];
            file.read_exact(&mut record_header).map_err(route_error)?;
            let raw_len =
                u32::from_le_bytes(record_header[..4].try_into().expect("fixed slice")) as usize;
            let expected_raw_sha256 = Digest(record_header[4..36].try_into().expect("fixed slice"));
            if raw_len == 0 || raw_len > MAXIMUM_RECORD_BYTES {
                return Err(route_message(
                    "campaign learner-head record size is invalid",
                ));
            }
            let record_bytes = RECORD_HEADER_BYTES
                .checked_add(raw_len)
                .ok_or_else(|| route_message("campaign learner-head record size overflowed"))?;
            if remaining < record_bytes as u64 {
                break;
            }
            let mut raw = vec![0_u8; raw_len];
            file.read_exact(&mut raw).map_err(route_error)?;
            if sha256(&raw) != expected_raw_sha256 {
                return Err(route_message(
                    "campaign learner-head record checksum is invalid",
                ));
            }
            let record: StoredCampaignLearnerHead =
                serde_cbor::from_slice(&raw).map_err(route_error)?;
            if record.schema != LEARNER_HEAD_SCHEMA_V1
                || record.sequence != records.len() as u64
                || record.replay_identity_sha256 != replay_identity_sha256
                || record.learner_snapshot_sha256 == Digest::ZERO
                || record.parent_head_sha256 != parent_head_sha256
                || record.head_sha256 != record.expected_head_sha256()?
            {
                return Err(route_message(
                    "campaign learner-head record authority is invalid",
                ));
            }
            if let Some(prior) = records.last()
                && (record.replay_revision < prior.replay_revision
                    || record.model_revision <= prior.model_revision)
            {
                return Err(route_message(
                    "campaign learner-head revisions are not monotonic",
                ));
            }
            parent_head_sha256 = record.head_sha256;
            records.push(record);
            valid_len = valid_len.saturating_add(record_bytes as u64);
        }
        if valid_len != file_len {
            file.set_len(valid_len).map_err(route_error)?;
            file.sync_all().map_err(route_error)?;
        }
        file.seek(SeekFrom::End(0)).map_err(route_error)?;
        Ok(Self {
            file,
            replay_identity_sha256,
            records,
        })
    }

    pub(super) fn latest(&self) -> Option<CampaignLearnerHead> {
        self.records.last().map(StoredCampaignLearnerHead::public)
    }

    pub(super) fn snapshot_sha256s(&self) -> impl Iterator<Item = Digest> + '_ {
        self.records
            .iter()
            .map(|record| record.learner_snapshot_sha256)
    }

    pub(super) fn publish(
        &mut self,
        head: CampaignLearnerHead,
    ) -> Result<bool, NativeTacticRouteRunError> {
        if self.latest() == Some(head) {
            return Ok(false);
        }
        if head.learner_snapshot_sha256 == Digest::ZERO {
            return Err(route_message(
                "campaign learner head has no snapshot identity",
            ));
        }
        if self.records.len() >= MAXIMUM_RECORDS {
            return Err(route_message(
                "campaign learner-head journal exceeds its record bound",
            ));
        }
        if let Some(prior) = self.latest()
            && (head.replay_revision < prior.replay_revision
                || head.model_revision <= prior.model_revision)
        {
            return Err(route_message("campaign learner head would move backward"));
        }
        let mut record = StoredCampaignLearnerHead {
            schema: LEARNER_HEAD_SCHEMA_V1.into(),
            sequence: self.records.len() as u64,
            replay_identity_sha256: self.replay_identity_sha256,
            learner_snapshot_sha256: head.learner_snapshot_sha256,
            replay_revision: head.replay_revision,
            model_revision: head.model_revision,
            parent_head_sha256: self
                .records
                .last()
                .map_or(Digest::ZERO, |prior| prior.head_sha256),
            head_sha256: Digest::ZERO,
        };
        record.head_sha256 = record.expected_head_sha256()?;
        let raw = serde_cbor::to_vec(&record).map_err(route_error)?;
        if raw.is_empty() || raw.len() > MAXIMUM_RECORD_BYTES {
            return Err(route_message("campaign learner-head record is oversized"));
        }
        let mut envelope = Vec::with_capacity(RECORD_HEADER_BYTES + raw.len());
        envelope.extend_from_slice(&u32::try_from(raw.len()).map_err(route_error)?.to_le_bytes());
        envelope.extend_from_slice(&sha256(&raw).0);
        envelope.extend_from_slice(&raw);
        self.file.write_all(&envelope).map_err(route_error)?;
        self.file.sync_data().map_err(route_error)?;
        self.records.push(record);
        Ok(true)
    }
}

fn journal_path(replay: &TacticReplayControlPlane) -> PathBuf {
    let mut path = OsString::from(replay.journal_path().as_os_str());
    path.push(".learner-head");
    PathBuf::from(path)
}

fn write_header(
    file: &mut File,
    replay_identity_sha256: Digest,
) -> Result<(), NativeTacticRouteRunError> {
    let mut header = Vec::with_capacity(JOURNAL_HEADER_BYTES);
    header.extend_from_slice(JOURNAL_MAGIC);
    header.extend_from_slice(&JOURNAL_VERSION.to_le_bytes());
    header.extend_from_slice(&0_u16.to_le_bytes());
    header.extend_from_slice(&replay_identity_sha256.0);
    file.write_all(&header).map_err(route_error)?;
    file.sync_all().map_err(route_error)?;
    Ok(())
}

fn sha256(bytes: &[u8]) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Digest(hasher.finalize().into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tactic_replay_control_plane::TacticReplayControlPlaneIdentity;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn learner_head_recovers_partial_tail_and_rejects_complete_corruption() {
        let root = std::env::temp_dir().join(format!(
            "dusklight-learner-head-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let identity = TacticReplayControlPlaneIdentity::new(
            Digest([1; 32]),
            Digest([2; 32]),
            Digest([3; 32]),
            Digest([4; 32]),
        )
        .unwrap();
        let replay = TacticReplayControlPlane::create(
            root.join("replay.dtrp"),
            root.join("objects"),
            identity,
        )
        .unwrap();
        let path = journal_path(&replay);
        let first = CampaignLearnerHead {
            learner_snapshot_sha256: Digest([5; 32]),
            replay_revision: 0,
            model_revision: 0,
        };
        let mut journal = CampaignLearnerHeadJournal::open_or_create(&replay).unwrap();
        assert!(journal.publish(first).unwrap());
        assert_eq!(journal.latest(), Some(first));
        drop(journal);

        let complete_len = fs::metadata(&path).unwrap().len();
        let mut partial = OpenOptions::new().append(true).open(&path).unwrap();
        partial.write_all(&[0xa5, 0x5a, 0x11]).unwrap();
        partial.sync_all().unwrap();
        drop(partial);
        let mut recovered = CampaignLearnerHeadJournal::open_or_create(&replay).unwrap();
        assert_eq!(recovered.latest(), Some(first));
        assert_eq!(fs::metadata(&path).unwrap().len(), complete_len);

        let second = CampaignLearnerHead {
            learner_snapshot_sha256: Digest([6; 32]),
            replay_revision: 1,
            model_revision: 1,
        };
        assert!(recovered.publish(second).unwrap());
        drop(recovered);
        let mut corrupt = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        corrupt.seek(SeekFrom::End(-1)).unwrap();
        let mut byte = [0_u8; 1];
        corrupt.read_exact(&mut byte).unwrap();
        corrupt.seek(SeekFrom::End(-1)).unwrap();
        corrupt.write_all(&[byte[0] ^ 0xff]).unwrap();
        corrupt.sync_all().unwrap();
        drop(corrupt);
        assert!(CampaignLearnerHeadJournal::open_or_create(&replay).is_err());

        drop(replay);
        fs::remove_dir_all(root).unwrap();
    }
}
