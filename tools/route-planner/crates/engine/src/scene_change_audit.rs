//! Reproducible source census for indexed SCLS and direct scene-change consumers.

use crate::artifact::Digest;
use crate::identity::ContentIdentity;
use crate::{PlannerContractError, canonical_json, require_canonical_json_bytes};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

pub const SCENE_CHANGE_CONSUMER_AUDIT_SCHEMA: &str =
    "dusklight.route-planner.scene-change-consumer-audit/v1";
const MAX_SOURCE_FILES: usize = 100_000;
const MAX_CALL_SITES: usize = 1_000_000;
const BUNDLED_GZ2E01_SCENE_CHANGE_CONSUMER_AUDIT: &[u8] =
    include_bytes!("../data/gz2e01-scene-change-consumer-audit.json");

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SceneChangeConsumerAudit {
    pub schema: String,
    pub content: ContentIdentity,
    pub source_files: Vec<SceneChangeSourceFile>,
    pub counts: Vec<SceneChangeConsumerCount>,
    pub content_sha256: Digest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SceneChangeSourceFile {
    pub relative_path: String,
    pub source_sha256: Digest,
    pub call_sites: Vec<SceneChangeCallSite>,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SceneChangeCallSite {
    pub line: u64,
    pub symbol: String,
    pub consumer_kind: SceneChangeConsumerKind,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SceneChangeConsumerKind {
    IndexedScls,
    CollisionIndexedScls,
    EventIndexedScls,
    PlayerLatchedScls,
    PlayerLatchedJumpScls,
    DirectDestination,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SceneChangeConsumerCount {
    pub consumer_kind: SceneChangeConsumerKind,
    pub call_sites: u64,
    pub source_files: u64,
}

const SYMBOLS: [(&str, SceneChangeConsumerKind); 6] = [
    (
        "dStage_changeSceneExitId",
        SceneChangeConsumerKind::CollisionIndexedScls,
    ),
    (
        "dStage_changeScene4Event",
        SceneChangeConsumerKind::EventIndexedScls,
    ),
    ("dStage_changeScene", SceneChangeConsumerKind::IndexedScls),
    (
        "onSceneChangeAreaJump",
        SceneChangeConsumerKind::PlayerLatchedJumpScls,
    ),
    (
        "onSceneChangeArea",
        SceneChangeConsumerKind::PlayerLatchedScls,
    ),
    (
        "dComIfGp_setNextStage",
        SceneChangeConsumerKind::DirectDestination,
    ),
];

impl SceneChangeConsumerAudit {
    pub fn extract(
        source_root: &Path,
        content: ContentIdentity,
    ) -> Result<Self, PlannerContractError> {
        content.validate()?;
        let root = source_root.canonicalize().map_err(|error| {
            PlannerContractError::new("scene_change_audit.source_root", error.to_string())
        })?;
        if !root.is_dir() {
            return Err(PlannerContractError::new(
                "scene_change_audit.source_root",
                "must resolve to a source directory",
            ));
        }
        let mut paths = Vec::new();
        collect_cpp_files(&root, &root, &mut paths)?;
        paths.sort();
        if paths.is_empty() || paths.len() > MAX_SOURCE_FILES {
            return Err(PlannerContractError::new(
                "scene_change_audit.source_files",
                format!("must inspect between 1 and {MAX_SOURCE_FILES} C++ source files"),
            ));
        }
        let mut source_files = Vec::new();
        let mut total_sites = 0_usize;
        for path in paths {
            let bytes = fs::read(&path).map_err(|error| {
                PlannerContractError::new("scene_change_audit.source_file", error.to_string())
            })?;
            let source = std::str::from_utf8(&bytes).map_err(|error| {
                PlannerContractError::new("scene_change_audit.source_file", error.to_string())
            })?;
            let call_sites = find_call_sites(source);
            if call_sites.is_empty() {
                continue;
            }
            total_sites = total_sites.checked_add(call_sites.len()).ok_or_else(|| {
                PlannerContractError::new("scene_change_audit.call_sites", "count overflowed")
            })?;
            if total_sites > MAX_CALL_SITES {
                return Err(PlannerContractError::new(
                    "scene_change_audit.call_sites",
                    format!("must contain at most {MAX_CALL_SITES} call sites"),
                ));
            }
            source_files.push(SceneChangeSourceFile {
                relative_path: relative_path(&root, &path)?,
                source_sha256: sha256(&bytes),
                call_sites,
            });
        }
        if source_files.is_empty() {
            return Err(PlannerContractError::new(
                "scene_change_audit.source_files",
                "contains no recognized scene-change consumers",
            ));
        }
        let counts = derive_counts(&source_files)?;
        let mut audit = Self {
            schema: SCENE_CHANGE_CONSUMER_AUDIT_SCHEMA.into(),
            content,
            source_files,
            counts,
            content_sha256: Digest::ZERO,
        };
        audit.content_sha256 = audit.identity()?;
        audit.validate()?;
        Ok(audit)
    }

    pub fn validate(&self) -> Result<(), PlannerContractError> {
        self.content.validate()?;
        if self.schema != SCENE_CHANGE_CONSUMER_AUDIT_SCHEMA
            || self.source_files.is_empty()
            || self.source_files.len() > MAX_SOURCE_FILES
            || self.content_sha256 == Digest::ZERO
            || self.content_sha256 != self.identity()?
        {
            return Err(PlannerContractError::new(
                "scene_change_audit",
                "has an unsupported schema, shape, or content seal",
            ));
        }
        let mut previous_path = None;
        let mut total = 0_usize;
        for source in &self.source_files {
            validate_relative_path(&source.relative_path)?;
            if source.source_sha256 == Digest::ZERO || source.call_sites.is_empty() {
                return Err(PlannerContractError::new(
                    "scene_change_audit.source_files",
                    "must bind nonempty call sites to a nonzero source digest",
                ));
            }
            if previous_path.is_some_and(|path: &str| path >= source.relative_path.as_str()) {
                return Err(PlannerContractError::new(
                    "scene_change_audit.source_files",
                    "must be unique and sorted by relative path",
                ));
            }
            if source.call_sites.windows(2).any(|pair| pair[0] >= pair[1])
                || source.call_sites.iter().any(|site| {
                    site.line == 0 || !symbol_matches_kind(&site.symbol, site.consumer_kind)
                })
            {
                return Err(PlannerContractError::new(
                    "scene_change_audit.call_sites",
                    "must be sorted exact recognized symbol occurrences on nonzero lines",
                ));
            }
            total = total.checked_add(source.call_sites.len()).ok_or_else(|| {
                PlannerContractError::new("scene_change_audit.call_sites", "count overflowed")
            })?;
            previous_path = Some(source.relative_path.as_str());
        }
        if total > MAX_CALL_SITES || self.counts != derive_counts(&self.source_files)? {
            return Err(PlannerContractError::new(
                "scene_change_audit.counts",
                "does not reproduce the exact source-file call sites",
            ));
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PlannerContractError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, PlannerContractError> {
        let audit: Self = serde_json::from_slice(bytes)?;
        audit.validate()?;
        require_canonical_json_bytes("scene_change_audit", bytes, &audit.canonical_bytes()?)?;
        Ok(audit)
    }

    fn identity(&self) -> Result<Digest, PlannerContractError> {
        let mut canonical = self.clone();
        canonical.content_sha256 = Digest::ZERO;
        Ok(sha256(&canonical_json(&canonical)?))
    }
}

pub fn bundled_gz2e01_scene_change_consumer_audit()
-> Result<SceneChangeConsumerAudit, PlannerContractError> {
    let audit =
        SceneChangeConsumerAudit::decode_canonical(BUNDLED_GZ2E01_SCENE_CHANGE_CONSUMER_AUDIT)?;
    if audit.content.id != "gcn-us-1.0-gz2e01" {
        return Err(PlannerContractError::new(
            "scene_change_audit.content",
            "bundled audit is not bound to exact GZ2E01 content",
        ));
    }
    Ok(audit)
}

fn collect_cpp_files(
    root: &Path,
    directory: &Path,
    output: &mut Vec<PathBuf>,
) -> Result<(), PlannerContractError> {
    let mut entries = fs::read_dir(directory)
        .map_err(|error| {
            PlannerContractError::new("scene_change_audit.source_root", error.to_string())
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            PlannerContractError::new("scene_change_audit.source_root", error.to_string())
        })?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            PlannerContractError::new("scene_change_audit.source_file", error.to_string())
        })?;
        if metadata.file_type().is_symlink() {
            return Err(PlannerContractError::new(
                "scene_change_audit.source_file",
                "source traversal rejects symbolic links",
            ));
        }
        if metadata.is_dir() {
            collect_cpp_files(root, &path, output)?;
        } else if metadata.is_file()
            && path.extension().and_then(|value| value.to_str()) == Some("cpp")
        {
            if output.len() == MAX_SOURCE_FILES {
                return Err(PlannerContractError::new(
                    "scene_change_audit.source_files",
                    format!("must contain at most {MAX_SOURCE_FILES} C++ source files"),
                ));
            }
            output.push(path);
        }
    }
    let _ = root;
    Ok(())
}

fn find_call_sites(source: &str) -> Vec<SceneChangeCallSite> {
    let source = strip_comments(source);
    let mut sites = Vec::new();
    for (line_index, line) in source.lines().enumerate() {
        for (symbol, kind) in SYMBOLS {
            let needle = format!("{symbol}(");
            for _ in line.match_indices(&needle) {
                sites.push(SceneChangeCallSite {
                    line: line_index as u64 + 1,
                    symbol: symbol.into(),
                    consumer_kind: kind,
                });
            }
        }
    }
    sites.sort();
    sites
}

fn strip_comments(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    let mut block = false;
    while index < bytes.len() {
        if block {
            if bytes.get(index..index + 2) == Some(b"*/") {
                output.extend_from_slice(b"  ");
                block = false;
                index += 2;
            } else {
                output.push(if bytes[index] == b'\n' { b'\n' } else { b' ' });
                index += 1;
            }
        } else if bytes.get(index..index + 2) == Some(b"/*") {
            output.extend_from_slice(b"  ");
            block = true;
            index += 2;
        } else if bytes.get(index..index + 2) == Some(b"//") {
            while index < bytes.len() && bytes[index] != b'\n' {
                output.push(b' ');
                index += 1;
            }
        } else {
            output.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(output).expect("comment stripping preserves UTF-8 bytes")
}

fn derive_counts(
    files: &[SceneChangeSourceFile],
) -> Result<Vec<SceneChangeConsumerCount>, PlannerContractError> {
    let mut counts = BTreeMap::<SceneChangeConsumerKind, (u64, u64)>::new();
    for source in files {
        let mut file_kinds = BTreeMap::<SceneChangeConsumerKind, u64>::new();
        for site in &source.call_sites {
            *file_kinds.entry(site.consumer_kind).or_default() += 1;
        }
        for (kind, sites) in file_kinds {
            let row = counts.entry(kind).or_default();
            row.0 = row.0.checked_add(sites).ok_or_else(|| {
                PlannerContractError::new("scene_change_audit.counts", "call-site count overflowed")
            })?;
            row.1 = row.1.checked_add(1).ok_or_else(|| {
                PlannerContractError::new(
                    "scene_change_audit.counts",
                    "source-file count overflowed",
                )
            })?;
        }
    }
    Ok(counts
        .into_iter()
        .map(
            |(consumer_kind, (call_sites, source_files))| SceneChangeConsumerCount {
                consumer_kind,
                call_sites,
                source_files,
            },
        )
        .collect())
}

fn symbol_matches_kind(symbol: &str, kind: SceneChangeConsumerKind) -> bool {
    SYMBOLS
        .iter()
        .any(|(expected, expected_kind)| symbol == *expected && kind == *expected_kind)
}

fn relative_path(root: &Path, path: &Path) -> Result<String, PlannerContractError> {
    let relative = path.strip_prefix(root).map_err(|_| {
        PlannerContractError::new("scene_change_audit.source_file", "escaped source root")
    })?;
    let value = relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/");
    validate_relative_path(&value)?;
    Ok(value)
}

fn validate_relative_path(value: &str) -> Result<(), PlannerContractError> {
    let path = Path::new(value);
    if value.is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(PlannerContractError::new(
            "scene_change_audit.source_file.relative_path",
            "must be a normalized relative path",
        ));
    }
    Ok(())
}

fn sha256(bytes: &[u8]) -> Digest {
    Digest(Sha256::digest(bytes).into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{ContentFingerprint, GamePlatform, GameRegion};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn content() -> ContentIdentity {
        ContentIdentity::new(
            "fixture",
            ContentFingerprint {
                platform: GamePlatform::GameCube,
                region: GameRegion::Usa,
                revision: "1.0".into(),
                product_id: "GZ2E01".into(),
                executable_sha256: Digest([1; 32]),
                game_data_sha256: Digest([2; 32]),
                resource_manifest_sha256: Digest([3; 32]),
            },
        )
        .unwrap()
    }

    #[test]
    fn extraction_seals_distinct_indexed_latched_event_and_direct_consumers() {
        let root = std::env::temp_dir().join(format!(
            "scene-change-audit-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("src/d/actor")).unwrap();
        fs::write(
            root.join("src/d/actor/door.cpp"),
            b"// dStage_changeScene(99)\nvoid f(){ player->onSceneChangeArea(2, 0, this); dStage_changeScene(2, 0, 0, 1, 0, -1); }\n",
        )
        .unwrap();
        fs::write(
            root.join("src/d/event.cpp"),
            b"void e(){ dStage_changeScene4Event(1, 0, 0, false, 0, 0, 0, -1); }\n/* dComIfGp_setNextStage(\"bad\",0,0,0); */\nvoid d(){ dComIfGp_setNextStage(\"good\",0,0,0); }\n",
        )
        .unwrap();
        let audit = SceneChangeConsumerAudit::extract(&root, content()).unwrap();
        assert_eq!(audit.source_files.len(), 2);
        assert_eq!(
            audit.counts.iter().map(|row| row.call_sites).sum::<u64>(),
            4
        );
        assert_eq!(
            SceneChangeConsumerAudit::decode_canonical(&audit.canonical_bytes().unwrap()).unwrap(),
            audit
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn altered_counts_sources_and_seals_fail_closed() {
        let sites = find_call_sites("dStage_changeScene(1); // onSceneChangeArea(2)\n");
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].consumer_kind, SceneChangeConsumerKind::IndexedScls);
    }

    #[test]
    fn bundled_gz2e01_audit_is_canonical_and_has_the_reproduced_census() {
        let audit = bundled_gz2e01_scene_change_consumer_audit().unwrap();
        assert_eq!(audit.source_files.len(), 68);
        assert_eq!(
            audit.counts.iter().map(|row| row.call_sites).sum::<u64>(),
            138
        );
        assert_eq!(
            audit.canonical_bytes().unwrap(),
            BUNDLED_GZ2E01_SCENE_CHANGE_CONSUMER_AUDIT
        );
    }
}
