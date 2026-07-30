//! Versioned binary persistence for replay-derived tactic macro lifecycle state.

use dusklight_automation_contracts::artifact::Digest;
use dusklight_automation_contracts::tape::InputTape;
use dusklight_learning::tactic_macro_promotion::{
    DiscoveredMacroCandidate, MacroComparisonEvidence, MacroEntryObservation, MacroPromotionStatus,
    MacroSourceProvenance, TACTIC_MACRO_DISCOVERY_SCHEMA_V4, TacticMacroComponent,
    TacticMacroPromotionRegistry,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

pub const TACTIC_MACRO_REGISTRY_EXTENSION: &str = "dtmr";
const TACTIC_MACRO_REGISTRY_SCHEMA_V4: &str = "dusklight-tactic-macro-registry/v4";
const TACTIC_MACRO_REGISTRY_MAGIC: &[u8; 8] = b"DSKTMAC4";
const TACTIC_MACRO_REGISTRY_VERSION: u16 = 4;
const TACTIC_MACRO_REGISTRY_HEADER_BYTES: usize = 8 + 2 + 2 + 8 + 8 + 32;
const TACTIC_MACRO_REGISTRY_COMPRESSION_LEVEL: i32 = 3;
const MAXIMUM_TACTIC_MACRO_REGISTRY_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct TacticMacroRegistryArtifact {
    pub content_sha256: Digest,
    pub registry: TacticMacroPromotionRegistry,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredRegistry {
    schema: String,
    discovery_schema: String,
    records: Vec<StoredRecord>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredRecord {
    candidate_sha256: Digest,
    option_id: String,
    tape: Vec<u8>,
    components: Vec<TacticMacroComponent>,
    sources: Vec<StoredSource>,
    comparisons: Vec<StoredComparison>,
    status: StoredStatus,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredSource {
    seed: u64,
    frontier_state_sha256: Digest,
    transition_sha256s: Vec<Digest>,
    stage: String,
    room: i8,
    player_procedure: Option<u16>,
    player_contacts: Option<u8>,
    goal_distance_f32_bits: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredComparison {
    comparison_sha256: Digest,
    candidate_sha256: Digest,
    seed: u64,
    frontier_state_sha256: Digest,
    candidate_terminal: bool,
    candidate_progress: f32,
    candidate_ticks: u32,
    primitive_terminal: bool,
    primitive_progress: f32,
    primitive_ticks: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum StoredStatus {
    Proposed,
    Promoted,
    Demoted,
}

pub fn write_tactic_macro_registry(
    path: &Path,
    registry: &TacticMacroPromotionRegistry,
) -> Result<Digest, TacticMacroStoreError> {
    if path.extension().and_then(|value| value.to_str()) != Some(TACTIC_MACRO_REGISTRY_EXTENSION) {
        return Err(store_error("tactic macro registry extension is invalid"));
    }
    let stored = StoredRegistry {
        schema: TACTIC_MACRO_REGISTRY_SCHEMA_V4.into(),
        discovery_schema: TACTIC_MACRO_DISCOVERY_SCHEMA_V4.into(),
        records: registry
            .records()
            .map(|record| {
                Ok(StoredRecord {
                    candidate_sha256: record.candidate.candidate_sha256,
                    option_id: record.candidate.option_id.clone(),
                    tape: record
                        .candidate
                        .tape
                        .encode()
                        .map_err(TacticMacroStoreError::domain)?,
                    components: record.candidate.components.clone(),
                    sources: record
                        .candidate
                        .sources
                        .iter()
                        .map(|source| StoredSource {
                            seed: source.seed,
                            frontier_state_sha256: source.frontier_state_sha256,
                            transition_sha256s: source.transition_sha256s.clone(),
                            stage: source.entry.stage.clone(),
                            room: source.entry.room,
                            player_procedure: source.entry.player_procedure,
                            player_contacts: source.entry.player_contacts,
                            goal_distance_f32_bits: source.entry.goal_distance_f32_bits,
                        })
                        .collect(),
                    comparisons: record
                        .comparisons
                        .iter()
                        .map(|comparison| StoredComparison {
                            comparison_sha256: comparison.comparison_sha256,
                            candidate_sha256: comparison.candidate_sha256,
                            seed: comparison.seed,
                            frontier_state_sha256: comparison.frontier_state_sha256,
                            candidate_terminal: comparison.candidate_terminal,
                            candidate_progress: comparison.candidate_progress,
                            candidate_ticks: comparison.candidate_ticks,
                            primitive_terminal: comparison.primitive_terminal,
                            primitive_progress: comparison.primitive_progress,
                            primitive_ticks: comparison.primitive_ticks,
                        })
                        .collect(),
                    status: record.status.into(),
                })
            })
            .collect::<Result<Vec<_>, TacticMacroStoreError>>()?,
    };
    let raw = serde_cbor::to_vec(&stored).map_err(TacticMacroStoreError::domain)?;
    let (content_sha256, envelope) = encode_registry(&raw)?;
    install_new(path, &envelope)?;
    Ok(content_sha256)
}

pub fn read_tactic_macro_registry(
    path: &Path,
) -> Result<TacticMacroRegistryArtifact, TacticMacroStoreError> {
    let metadata = fs::symlink_metadata(path).map_err(TacticMacroStoreError::io)?;
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() < TACTIC_MACRO_REGISTRY_HEADER_BYTES as u64
        || metadata.len()
            > (MAXIMUM_TACTIC_MACRO_REGISTRY_BYTES + TACTIC_MACRO_REGISTRY_HEADER_BYTES) as u64
    {
        return Err(store_error(
            "tactic macro registry is not a bounded physical file",
        ));
    }
    let bytes = fs::read(path).map_err(TacticMacroStoreError::io)?;
    let (content_sha256, raw) = decode_registry(&bytes)?;
    let stored: StoredRegistry =
        serde_cbor::from_slice(&raw).map_err(TacticMacroStoreError::domain)?;
    if stored.schema != TACTIC_MACRO_REGISTRY_SCHEMA_V4
        || stored.discovery_schema != TACTIC_MACRO_DISCOVERY_SCHEMA_V4
    {
        return Err(store_error("tactic macro registry schema is invalid"));
    }
    let mut registry = TacticMacroPromotionRegistry::default();
    for record in stored.records {
        let decoded = InputTape::decode(&record.tape)
            .map_err(TacticMacroStoreError::domain)?
            .tape;
        let candidate = DiscoveredMacroCandidate {
            candidate_sha256: record.candidate_sha256,
            option_id: record.option_id,
            tape: decoded,
            components: record.components,
            sources: record
                .sources
                .into_iter()
                .map(|source| MacroSourceProvenance {
                    seed: source.seed,
                    frontier_state_sha256: source.frontier_state_sha256,
                    transition_sha256s: source.transition_sha256s,
                    entry: MacroEntryObservation {
                        stage: source.stage,
                        room: source.room,
                        player_procedure: source.player_procedure,
                        player_contacts: source.player_contacts,
                        goal_distance_f32_bits: source.goal_distance_f32_bits,
                    },
                })
                .collect(),
        };
        registry
            .propose(candidate)
            .map_err(TacticMacroStoreError::domain)?;
        for stored_comparison in record.comparisons {
            let comparison = MacroComparisonEvidence::new(
                stored_comparison.candidate_sha256,
                stored_comparison.seed,
                stored_comparison.frontier_state_sha256,
                stored_comparison.candidate_terminal,
                stored_comparison.candidate_progress,
                stored_comparison.candidate_ticks,
                stored_comparison.primitive_terminal,
                stored_comparison.primitive_progress,
                stored_comparison.primitive_ticks,
            )
            .map_err(TacticMacroStoreError::domain)?;
            if comparison.comparison_sha256 != stored_comparison.comparison_sha256 {
                return Err(store_error("stored macro comparison identity is detached"));
            }
            registry
                .observe(comparison)
                .map_err(TacticMacroStoreError::domain)?;
        }
        let restored = registry
            .records()
            .find(|candidate| candidate.candidate.candidate_sha256 == record.candidate_sha256)
            .ok_or_else(|| store_error("restored macro record is absent"))?;
        if restored.status != record.status.into() {
            return Err(store_error(
                "stored macro lifecycle status is detached from its evidence",
            ));
        }
    }
    Ok(TacticMacroRegistryArtifact {
        content_sha256,
        registry,
    })
}

fn encode_registry(raw: &[u8]) -> Result<(Digest, Vec<u8>), TacticMacroStoreError> {
    if raw.is_empty() || raw.len() > MAXIMUM_TACTIC_MACRO_REGISTRY_BYTES {
        return Err(store_error("tactic macro registry payload is invalid"));
    }
    let compressed = zstd::bulk::compress(raw, TACTIC_MACRO_REGISTRY_COMPRESSION_LEVEL)
        .map_err(TacticMacroStoreError::domain)?;
    let content_sha256 = sha256(raw);
    let mut envelope = Vec::with_capacity(TACTIC_MACRO_REGISTRY_HEADER_BYTES + compressed.len());
    envelope.extend_from_slice(TACTIC_MACRO_REGISTRY_MAGIC);
    envelope.extend_from_slice(&TACTIC_MACRO_REGISTRY_VERSION.to_le_bytes());
    envelope.extend_from_slice(&0_u16.to_le_bytes());
    envelope.extend_from_slice(&(raw.len() as u64).to_le_bytes());
    envelope.extend_from_slice(&(compressed.len() as u64).to_le_bytes());
    envelope.extend_from_slice(&content_sha256.0);
    envelope.extend_from_slice(&compressed);
    Ok((content_sha256, envelope))
}

fn decode_registry(bytes: &[u8]) -> Result<(Digest, Vec<u8>), TacticMacroStoreError> {
    if bytes.len() < TACTIC_MACRO_REGISTRY_HEADER_BYTES
        || &bytes[..8] != TACTIC_MACRO_REGISTRY_MAGIC
        || u16::from_le_bytes([bytes[8], bytes[9]]) != TACTIC_MACRO_REGISTRY_VERSION
        || u16::from_le_bytes([bytes[10], bytes[11]]) != 0
    {
        return Err(store_error("tactic macro registry envelope is invalid"));
    }
    let raw_len = u64::from_le_bytes(bytes[12..20].try_into().unwrap()) as usize;
    let compressed_len = u64::from_le_bytes(bytes[20..28].try_into().unwrap()) as usize;
    if raw_len == 0
        || raw_len > MAXIMUM_TACTIC_MACRO_REGISTRY_BYTES
        || compressed_len == 0
        || compressed_len > MAXIMUM_TACTIC_MACRO_REGISTRY_BYTES
        || bytes.len() != TACTIC_MACRO_REGISTRY_HEADER_BYTES + compressed_len
    {
        return Err(store_error("tactic macro registry lengths are invalid"));
    }
    let expected = Digest(bytes[28..60].try_into().unwrap());
    let raw = zstd::bulk::decompress(&bytes[TACTIC_MACRO_REGISTRY_HEADER_BYTES..], raw_len)
        .map_err(TacticMacroStoreError::domain)?;
    if raw.len() != raw_len || sha256(&raw) != expected {
        return Err(store_error(
            "tactic macro registry content identity is detached",
        ));
    }
    Ok((expected, raw))
}

fn install_new(path: &Path, bytes: &[u8]) -> Result<(), TacticMacroStoreError> {
    if path.exists() {
        let existing = fs::read(path).map_err(TacticMacroStoreError::io)?;
        return if existing == bytes {
            Ok(())
        } else {
            Err(store_error(
                "tactic macro registry path contains different immutable content",
            ))
        };
    }
    let parent = path
        .parent()
        .ok_or_else(|| store_error("tactic macro registry has no parent"))?;
    fs::create_dir_all(parent).map_err(TacticMacroStoreError::io)?;
    let partial = parent.join(format!(
        ".{}.{}.partial",
        path.file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| store_error("tactic macro registry file name is invalid"))?,
        std::process::id()
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&partial)
        .map_err(TacticMacroStoreError::io)?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(TacticMacroStoreError::io)?;
    drop(file);
    fs::rename(&partial, path).map_err(TacticMacroStoreError::io)
}

fn sha256(bytes: &[u8]) -> Digest {
    Digest(Sha256::digest(bytes).into())
}

impl From<MacroPromotionStatus> for StoredStatus {
    fn from(value: MacroPromotionStatus) -> Self {
        match value {
            MacroPromotionStatus::Proposed => Self::Proposed,
            MacroPromotionStatus::Promoted => Self::Promoted,
            MacroPromotionStatus::Demoted => Self::Demoted,
        }
    }
}

impl From<StoredStatus> for MacroPromotionStatus {
    fn from(value: StoredStatus) -> Self {
        match value {
            StoredStatus::Proposed => Self::Proposed,
            StoredStatus::Promoted => Self::Promoted,
            StoredStatus::Demoted => Self::Demoted,
        }
    }
}

#[derive(Debug)]
pub struct TacticMacroStoreError(String);

impl TacticMacroStoreError {
    fn io(error: std::io::Error) -> Self {
        Self(error.to_string())
    }

    fn domain(error: impl fmt::Display) -> Self {
        Self(error.to_string())
    }
}

impl fmt::Display for TacticMacroStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for TacticMacroStoreError {}

fn store_error(message: impl Into<String>) -> TacticMacroStoreError {
    TacticMacroStoreError(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dusklight_automation_contracts::tape::InputFrame;
    use dusklight_learning::tactic_asset::{TacticAssetSource, TacticCatalogEntry};
    use dusklight_learning::tactic_macro_promotion::{
        MacroDiscoveryObservation, discover_replay_macros,
    };

    fn observation(seed: u64, identity: u8) -> MacroDiscoveryObservation {
        let mut frame = InputFrame::default();
        frame.owned_ports = 1;
        frame.pads[0].stick_x = 90;
        let tape = InputTape {
            frames: vec![frame; 8],
            ..InputTape::default()
        };
        let entry = TacticCatalogEntry::new(
            "family/seek",
            TacticAssetSource::RecordedTape(tape.clone()),
        )
        .unwrap();
        MacroDiscoveryObservation {
            seed,
            frontier_state_sha256: Digest([identity; 32]),
            transition_sha256: Digest([identity.saturating_add(16); 32]),
            component: TacticMacroComponent::from_catalog_entry(&entry).unwrap(),
            entry: MacroEntryObservation {
                stage: "F_SP103".into(),
                room: 1,
                player_procedure: Some(3),
                player_contacts: Some(1),
                goal_distance_f32_bits: (100.0 + f32::from(identity)).to_bits(),
            },
            tape,
            reward: 1.0,
            goal_progress: 8.0,
            terminal: false,
        }
    }

    #[test]
    fn binary_registry_round_trips_lifecycle_evidence() {
        let candidate =
            discover_replay_macros(&[observation(11, 1), observation(13, 2)]).unwrap()[0].clone();
        let mut registry = TacticMacroPromotionRegistry::default();
        registry.propose(candidate.clone()).unwrap();
        for (seed, state) in [(11, 1), (13, 2)] {
            registry
                .observe(
                    MacroComparisonEvidence::new(
                        candidate.candidate_sha256,
                        seed,
                        Digest([state; 32]),
                        false,
                        12.0,
                        8,
                        false,
                        8.0,
                        8,
                    )
                    .unwrap(),
                )
                .unwrap();
        }

        let root = std::env::temp_dir().join(format!(
            "dusklight-tactic-macro-registry-{}-{}",
            std::process::id(),
            candidate.option_id.replace('/', "-")
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join(format!("registry.{TACTIC_MACRO_REGISTRY_EXTENSION}"));
        let written = write_tactic_macro_registry(&path, &registry).unwrap();
        let restored = read_tactic_macro_registry(&path).unwrap();
        assert_eq!(restored.content_sha256, written);
        assert_eq!(restored.registry, registry);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn binary_registry_rejects_content_tampering() {
        let mut registry = TacticMacroPromotionRegistry::default();
        let candidate =
            discover_replay_macros(&[observation(11, 1), observation(13, 2)]).unwrap()[0].clone();
        registry.propose(candidate.clone()).unwrap();
        let root = std::env::temp_dir().join(format!(
            "dusklight-tactic-macro-tamper-{}-{}",
            std::process::id(),
            candidate.option_id.replace('/', "-")
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join(format!("registry.{TACTIC_MACRO_REGISTRY_EXTENSION}"));
        write_tactic_macro_registry(&path, &registry).unwrap();
        let mut bytes = fs::read(&path).unwrap();
        *bytes.last_mut().unwrap() ^= 0x80;
        fs::write(&path, bytes).unwrap();
        assert!(read_tactic_macro_registry(&path).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
