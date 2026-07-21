//! Authenticated compression for large stage-survey evidence files.
//!
//! The survey ledger always identifies the uncompressed native artifact. This
//! envelope is only a storage representation: readers must decompress and
//! reproduce the ledger-bound SHA-256 before accepting any bytes.

use dusklight_automation_contracts::artifact::Digest;
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 8] = b"DSKSVZ01";
const HEADER_SIZE: usize = 8 + 8 + 32;
const MAXIMUM_RAW_BYTES: u64 = 512 * 1024 * 1024;
const COMPRESSION_LEVEL: i32 = 1;

pub(crate) fn compressed_artifact_path(path: &Path) -> PathBuf {
    let mut value: OsString = path.as_os_str().to_owned();
    value.push(".zst");
    PathBuf::from(value)
}

pub(crate) fn compact_survey_artifact(
    raw_path: &Path,
    expected_digest: Digest,
) -> Result<bool, StageSurveyArtifactError> {
    let compressed_path = compressed_artifact_path(raw_path);
    if !raw_path.is_file() {
        if compressed_path.is_file() {
            read_compressed_artifact(&compressed_path, expected_digest)?;
            return Ok(false);
        }
        return Err(StageSurveyArtifactError::new(format!(
            "survey artifact is missing: {}",
            raw_path.display()
        )));
    }
    let raw = fs::read(raw_path).map_err(StageSurveyArtifactError::io)?;
    verify_raw(&raw, expected_digest)?;
    let raw_len = u64::try_from(raw.len())
        .map_err(|_| StageSurveyArtifactError::new("survey artifact length overflowed"))?;
    if raw_len > MAXIMUM_RAW_BYTES {
        return Err(StageSurveyArtifactError::new(
            "survey artifact exceeds the compression size bound",
        ));
    }
    let compressed = zstd::bulk::compress(&raw, COMPRESSION_LEVEL)
        .map_err(|error| StageSurveyArtifactError::new(error.to_string()))?;
    let mut envelope = Vec::with_capacity(HEADER_SIZE + compressed.len());
    envelope.extend_from_slice(MAGIC);
    envelope.extend_from_slice(&raw_len.to_le_bytes());
    envelope.extend_from_slice(&expected_digest.0);
    envelope.extend_from_slice(&compressed);

    let temporary = temporary_path(&compressed_path);
    if temporary.exists() {
        fs::remove_file(&temporary).map_err(StageSurveyArtifactError::io)?;
    }
    let mut output = File::create(&temporary).map_err(StageSurveyArtifactError::io)?;
    output
        .write_all(&envelope)
        .map_err(StageSurveyArtifactError::io)?;
    output.sync_all().map_err(StageSurveyArtifactError::io)?;
    drop(output);
    if compressed_path.exists() {
        read_compressed_artifact(&compressed_path, expected_digest)?;
        fs::remove_file(&temporary).map_err(StageSurveyArtifactError::io)?;
    } else {
        fs::rename(&temporary, &compressed_path).map_err(StageSurveyArtifactError::io)?;
    }
    read_compressed_artifact(&compressed_path, expected_digest)?;
    fs::remove_file(raw_path).map_err(StageSurveyArtifactError::io)?;
    Ok(true)
}

pub(crate) fn read_survey_artifact(
    raw_path: &Path,
    expected_digest: Digest,
) -> Result<Option<Vec<u8>>, StageSurveyArtifactError> {
    if raw_path.is_file() {
        let raw = fs::read(raw_path).map_err(StageSurveyArtifactError::io)?;
        verify_raw(&raw, expected_digest)?;
        return Ok(Some(raw));
    }
    let compressed_path = compressed_artifact_path(raw_path);
    if compressed_path.is_file() {
        return read_compressed_artifact(&compressed_path, expected_digest).map(Some);
    }
    Ok(None)
}

fn read_compressed_artifact(
    path: &Path,
    expected_digest: Digest,
) -> Result<Vec<u8>, StageSurveyArtifactError> {
    let envelope = fs::read(path).map_err(StageSurveyArtifactError::io)?;
    if envelope.len() < HEADER_SIZE || &envelope[..8] != MAGIC {
        return Err(StageSurveyArtifactError::new(
            "survey artifact compression envelope is invalid",
        ));
    }
    let raw_len = u64::from_le_bytes(envelope[8..16].try_into().expect("fixed slice"));
    let sealed_digest = Digest(envelope[16..48].try_into().expect("fixed slice"));
    if raw_len > MAXIMUM_RAW_BYTES || sealed_digest != expected_digest {
        return Err(StageSurveyArtifactError::new(
            "survey artifact compression identity is invalid",
        ));
    }
    let raw_len = usize::try_from(raw_len)
        .map_err(|_| StageSurveyArtifactError::new("survey artifact length overflowed"))?;
    let raw = zstd::bulk::decompress(&envelope[HEADER_SIZE..], raw_len)
        .map_err(|error| StageSurveyArtifactError::new(error.to_string()))?;
    if raw.len() != raw_len {
        return Err(StageSurveyArtifactError::new(
            "survey artifact decompressed length is invalid",
        ));
    }
    verify_raw(&raw, expected_digest)?;
    Ok(raw)
}

fn verify_raw(bytes: &[u8], expected_digest: Digest) -> Result<(), StageSurveyArtifactError> {
    if Digest(Sha256::digest(bytes).into()) != expected_digest {
        return Err(StageSurveyArtifactError::new(
            "survey artifact content digest is invalid",
        ));
    }
    Ok(())
}

fn temporary_path(path: &Path) -> PathBuf {
    let mut value: OsString = path.as_os_str().to_owned();
    value.push(".next");
    PathBuf::from(value)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StageSurveyArtifactError(String);

impl StageSurveyArtifactError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }

    fn io(error: std::io::Error) -> Self {
        Self::new(error.to_string())
    }
}

impl fmt::Display for StageSurveyArtifactError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for StageSurveyArtifactError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_root() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("dusklight-survey-artifact-{nonce}"))
    }

    #[test]
    fn compression_preserves_raw_identity_and_rejects_tampering() {
        let root = temporary_root();
        fs::create_dir_all(&root).unwrap();
        let raw_path = root.join("observation.trace");
        let raw = vec![0x5a; 256 * 1024];
        let digest = Digest(Sha256::digest(&raw).into());
        fs::write(&raw_path, &raw).unwrap();

        assert!(compact_survey_artifact(&raw_path, digest).unwrap());
        assert!(!raw_path.exists());
        let compressed_path = compressed_artifact_path(&raw_path);
        assert!(compressed_path.metadata().unwrap().len() < raw.len() as u64 / 10);
        assert_eq!(read_survey_artifact(&raw_path, digest).unwrap(), Some(raw));
        assert!(!compact_survey_artifact(&raw_path, digest).unwrap());

        let mut bytes = fs::read(&compressed_path).unwrap();
        *bytes.last_mut().unwrap() ^= 1;
        fs::write(&compressed_path, bytes).unwrap();
        assert!(read_survey_artifact(&raw_path, digest).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
