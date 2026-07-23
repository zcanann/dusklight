//! Exact-content placement and source census for return/restart writers.

use crate::artifact::Digest;
use crate::orig_discovery::{ExtractedOrigBundle, ExtractedOrigStageArchive};
use crate::orig_extraction::ExtractedActorPlacement;
use crate::{canonical_json, require_canonical_json_bytes, PlannerContractError};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

pub const RETURN_RESTART_AUDIT_SCHEMA: &str = "dusklight.route-planner.return-restart-audit/v1";
const MAX_SOURCE_FILES: usize = 100_000;
const MAX_CALL_SITES: usize = 1_000_000;
const MAX_PLACEMENTS: usize = 1_000_000;
const BUNDLED_GZ2E01_RETURN_RESTART_AUDIT: &[u8] =
    include_bytes!("../data/gz2e01-return-restart-audit.json");

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReturnRestartAudit {
    pub schema: String,
    pub content: crate::identity::ContentIdentity,
    pub source_bundle_sha256: Digest,
    pub source_files: Vec<ReturnRestartSourceFile>,
    pub writer_counts: Vec<ReturnRestartWriterCount>,
    pub savmem_placements: Vec<SavmemPlacementAudit>,
    pub content_sha256: Digest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReturnRestartSourceFile {
    pub relative_path: String,
    pub source_sha256: Digest,
    pub call_sites: Vec<ReturnRestartCallSite>,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReturnRestartCallSite {
    pub line: u64,
    pub symbol: String,
    pub writer_kind: ReturnRestartWriterKind,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReturnRestartWriterKind {
    PlayerReturnPlaceInitialize,
    PlayerReturnPlaceSet,
    RestartPlaceSet,
    RestartRoomParameterSet,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReturnRestartWriterCount {
    pub writer_kind: ReturnRestartWriterKind,
    pub call_sites: u64,
    pub source_files: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SavmemPlacementAudit {
    pub archive_relative_path: String,
    pub archive_sha256: Digest,
    pub resource_name: String,
    pub resource_sha256: Digest,
    pub chunk_tag: String,
    pub record_index: u32,
    pub layer: Option<u8>,
    pub parameters: u32,
    pub position: [f32; 3],
    pub angle: [i16; 3],
    pub set_id: u16,
    pub raw_hex: String,
    pub target: SavmemTarget,
    pub guards: SavmemGuards,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SavmemTarget {
    pub stage_name: String,
    pub parameter_room_raw: u8,
    pub placement_room: Option<i8>,
    pub effective_save_room: i8,
    pub save_point: u8,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SavmemGuards {
    pub no_telop_must_be_unset: bool,
    pub event_1_must_be_set: Option<u16>,
    pub event_2_must_be_unset: Option<u16>,
    pub switch_1_must_be_set: Option<u8>,
    pub switch_2_must_be_unset: Option<u8>,
}

const WRITER_SYMBOLS: [(&str, ReturnRestartWriterKind); 4] = [
    (
        "getPlayerReturnPlace().set",
        ReturnRestartWriterKind::PlayerReturnPlaceSet,
    ),
    (
        "mPlayerReturnPlace.init",
        ReturnRestartWriterKind::PlayerReturnPlaceInitialize,
    ),
    (
        "dComIfGs_setRestartRoomParam",
        ReturnRestartWriterKind::RestartRoomParameterSet,
    ),
    (
        "dComIfGs_setRestartRoom",
        ReturnRestartWriterKind::RestartPlaceSet,
    ),
];

impl ReturnRestartAudit {
    pub fn extract(
        repository_root: &Path,
        bundle: &ExtractedOrigBundle,
    ) -> Result<Self, PlannerContractError> {
        bundle.validate()?;
        let root = repository_root.canonicalize().map_err(|error| {
            PlannerContractError::new("return_restart_audit.repository_root", error.to_string())
        })?;
        if !root.is_dir() {
            return Err(PlannerContractError::new(
                "return_restart_audit.repository_root",
                "must resolve to a directory",
            ));
        }
        let source_root = root.join("src");
        if !source_root.is_dir() {
            return Err(PlannerContractError::new(
                "return_restart_audit.repository_root",
                "must contain the source directory",
            ));
        }
        let mut paths = Vec::new();
        collect_source_files(&source_root, &mut paths)?;
        paths.sort();
        if paths.is_empty() || paths.len() > MAX_SOURCE_FILES {
            return Err(PlannerContractError::new(
                "return_restart_audit.source_files",
                format!("must inspect between 1 and {MAX_SOURCE_FILES} source files"),
            ));
        }
        let mut source_files = Vec::new();
        let mut total_sites = 0_usize;
        for path in paths {
            let bytes = fs::read(&path).map_err(|error| {
                PlannerContractError::new("return_restart_audit.source_file", error.to_string())
            })?;
            let source = std::str::from_utf8(&bytes).map_err(|error| {
                PlannerContractError::new("return_restart_audit.source_file", error.to_string())
            })?;
            let call_sites = find_call_sites(source);
            if call_sites.is_empty() {
                continue;
            }
            total_sites = total_sites.checked_add(call_sites.len()).ok_or_else(|| {
                PlannerContractError::new("return_restart_audit.call_sites", "count overflowed")
            })?;
            if total_sites > MAX_CALL_SITES {
                return Err(PlannerContractError::new(
                    "return_restart_audit.call_sites",
                    format!("must contain at most {MAX_CALL_SITES} call sites"),
                ));
            }
            source_files.push(ReturnRestartSourceFile {
                relative_path: relative_path(&root, &path)?,
                source_sha256: sha256(&bytes),
                call_sites,
            });
        }
        if source_files.is_empty() {
            return Err(PlannerContractError::new(
                "return_restart_audit.source_files",
                "contains no recognized return/restart writers",
            ));
        }
        let mut savmem_placements = Vec::new();
        for archive in &bundle.stages {
            for placement in &archive.stage.actor_placements {
                if placement.name == "Savmem" {
                    savmem_placements.push(decode_savmem_placement(archive, placement)?);
                }
            }
        }
        savmem_placements.sort_by(|left, right| placement_key(left).cmp(&placement_key(right)));
        if savmem_placements.is_empty() || savmem_placements.len() > MAX_PLACEMENTS {
            return Err(PlannerContractError::new(
                "return_restart_audit.savmem_placements",
                format!("must contain between 1 and {MAX_PLACEMENTS} placements"),
            ));
        }
        let mut audit = Self {
            schema: RETURN_RESTART_AUDIT_SCHEMA.into(),
            content: bundle.content.clone(),
            source_bundle_sha256: bundle.digest()?,
            writer_counts: derive_counts(&source_files)?,
            source_files,
            savmem_placements,
            content_sha256: Digest::ZERO,
        };
        audit.content_sha256 = audit.identity()?;
        audit.validate()?;
        Ok(audit)
    }

    pub fn validate(&self) -> Result<(), PlannerContractError> {
        self.content.validate()?;
        if self.schema != RETURN_RESTART_AUDIT_SCHEMA
            || self.source_bundle_sha256 == Digest::ZERO
            || self.content_sha256 == Digest::ZERO
            || self.content_sha256 != self.identity()?
            || self.source_files.is_empty()
            || self.source_files.len() > MAX_SOURCE_FILES
            || self.savmem_placements.is_empty()
            || self.savmem_placements.len() > MAX_PLACEMENTS
        {
            return Err(PlannerContractError::new(
                "return_restart_audit",
                "has an unsupported schema, shape, or content seal",
            ));
        }
        let mut previous_path = None;
        let mut total_sites = 0_usize;
        for source in &self.source_files {
            validate_relative_path(&source.relative_path)?;
            if source.source_sha256 == Digest::ZERO || source.call_sites.is_empty() {
                return Err(PlannerContractError::new(
                    "return_restart_audit.source_files",
                    "must bind nonempty call sites to nonzero source digests",
                ));
            }
            if previous_path.is_some_and(|path: &str| path >= source.relative_path.as_str())
                || source.call_sites.windows(2).any(|pair| pair[0] >= pair[1])
                || source.call_sites.iter().any(|site| {
                    site.line == 0 || !symbol_matches_kind(&site.symbol, site.writer_kind)
                })
            {
                return Err(PlannerContractError::new(
                    "return_restart_audit.call_sites",
                    "must be sorted recognized source occurrences",
                ));
            }
            total_sites = total_sites
                .checked_add(source.call_sites.len())
                .ok_or_else(|| {
                    PlannerContractError::new("return_restart_audit.call_sites", "count overflowed")
                })?;
            previous_path = Some(source.relative_path.as_str());
        }
        if total_sites > MAX_CALL_SITES || self.writer_counts != derive_counts(&self.source_files)?
        {
            return Err(PlannerContractError::new(
                "return_restart_audit.writer_counts",
                "does not reproduce the source census",
            ));
        }
        if self
            .savmem_placements
            .windows(2)
            .any(|pair| placement_key(&pair[0]) >= placement_key(&pair[1]))
        {
            return Err(PlannerContractError::new(
                "return_restart_audit.savmem_placements",
                "must be unique and sorted by exact resource record",
            ));
        }
        for placement in &self.savmem_placements {
            validate_placement(placement)?;
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
        require_canonical_json_bytes("return_restart_audit", bytes, &audit.canonical_bytes()?)?;
        Ok(audit)
    }

    fn identity(&self) -> Result<Digest, PlannerContractError> {
        let mut canonical = self.clone();
        canonical.content_sha256 = Digest::ZERO;
        Ok(sha256(&canonical_json(&canonical)?))
    }
}

pub fn bundled_gz2e01_return_restart_audit() -> Result<ReturnRestartAudit, PlannerContractError> {
    let audit = ReturnRestartAudit::decode_canonical(BUNDLED_GZ2E01_RETURN_RESTART_AUDIT)?;
    if audit.content.id != "gcn-us-1.0-gz2e01" {
        return Err(PlannerContractError::new(
            "return_restart_audit.content",
            "bundled audit is not bound to exact GZ2E01 content",
        ));
    }
    Ok(audit)
}

fn decode_savmem_placement(
    archive: &ExtractedOrigStageArchive,
    placement: &ExtractedActorPlacement,
) -> Result<SavmemPlacementAudit, PlannerContractError> {
    let (stage_name, placement_room) = stage_and_room(&archive.relative_path)?;
    let parameter_room_raw = ((placement.parameters >> 8) & 0xff) as u8;
    let encoded_room = parameter_room_raw as i8;
    let z = placement.angle[2] as u16;
    let optional_u16 = |value: i16| (value as u16 != u16::MAX).then_some(value as u16);
    let optional_u8 = |value: u8| (value != u8::MAX).then_some(value);
    Ok(SavmemPlacementAudit {
        archive_relative_path: archive.relative_path.clone(),
        archive_sha256: archive.archive_sha256,
        resource_name: archive.resource_name.clone(),
        resource_sha256: archive.resource_sha256,
        chunk_tag: placement.chunk_tag.clone(),
        record_index: placement.record_index,
        layer: placement.layer,
        parameters: placement.parameters,
        position: placement.position,
        angle: placement.angle,
        set_id: placement.set_id,
        raw_hex: placement.raw_hex.clone(),
        target: SavmemTarget {
            stage_name,
            parameter_room_raw,
            placement_room,
            effective_save_room: placement_room.unwrap_or(encoded_room),
            save_point: (placement.parameters & 0xff) as u8,
        },
        guards: SavmemGuards {
            no_telop_must_be_unset: true,
            event_1_must_be_set: optional_u16(placement.angle[0]),
            event_2_must_be_unset: optional_u16(placement.angle[1]),
            switch_1_must_be_set: optional_u8(z as u8),
            switch_2_must_be_unset: optional_u8((z >> 8) as u8),
        },
    })
}

fn validate_placement(placement: &SavmemPlacementAudit) -> Result<(), PlannerContractError> {
    validate_relative_path(&placement.archive_relative_path)?;
    let (stage_name, placement_room) = stage_and_room(&placement.archive_relative_path)?;
    if placement.archive_sha256 == Digest::ZERO
        || placement.resource_sha256 == Digest::ZERO
        || placement.resource_name.is_empty()
        || placement.chunk_tag.len() != 4
        || placement.position.iter().any(|value| !value.is_finite())
        || placement.raw_hex.len() != 64
        || !placement
            .raw_hex
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(PlannerContractError::new(
            "return_restart_audit.savmem_placements",
            "contains an invalid exact resource record",
        ));
    }
    let parameter_room_raw = ((placement.parameters >> 8) & 0xff) as u8;
    let encoded_room = parameter_room_raw as i8;
    let z = placement.angle[2] as u16;
    let expected_target = SavmemTarget {
        stage_name,
        parameter_room_raw,
        placement_room,
        effective_save_room: placement_room.unwrap_or(encoded_room),
        save_point: (placement.parameters & 0xff) as u8,
    };
    let expected_guards = SavmemGuards {
        no_telop_must_be_unset: true,
        event_1_must_be_set: (placement.angle[0] as u16 != u16::MAX)
            .then_some(placement.angle[0] as u16),
        event_2_must_be_unset: (placement.angle[1] as u16 != u16::MAX)
            .then_some(placement.angle[1] as u16),
        switch_1_must_be_set: (z as u8 != u8::MAX).then_some(z as u8),
        switch_2_must_be_unset: ((z >> 8) as u8 != u8::MAX).then_some((z >> 8) as u8),
    };
    if placement.target != expected_target || placement.guards != expected_guards {
        return Err(PlannerContractError::new(
            "return_restart_audit.savmem_placements",
            "decoded target or guards disagree with the exact placement bytes",
        ));
    }
    Ok(())
}

fn placement_key(placement: &SavmemPlacementAudit) -> (&str, &str, &str, Option<u8>, u32) {
    (
        &placement.archive_relative_path,
        &placement.resource_name,
        &placement.chunk_tag,
        placement.layer,
        placement.record_index,
    )
}

fn stage_and_room(relative_path: &str) -> Result<(String, Option<i8>), PlannerContractError> {
    validate_relative_path(relative_path)?;
    let parts = relative_path.split('/').collect::<Vec<_>>();
    if parts.len() != 5 || parts[..3] != ["files", "res", "Stage"] {
        return Err(PlannerContractError::new(
            "return_restart_audit.savmem_placements.archive_relative_path",
            "is not a canonical stage archive path",
        ));
    }
    let stage_name = parts[3];
    if stage_name.is_empty() || stage_name.len() > 8 || !stage_name.is_ascii() {
        return Err(PlannerContractError::new(
            "return_restart_audit.savmem_placements.stage_name",
            "is not a bounded stage name",
        ));
    }
    let archive_name = parts[4];
    let room = if archive_name.starts_with('R')
        && archive_name
            .as_bytes()
            .get(1..3)
            .is_some_and(|value| value.len() == 2 && value.iter().all(|byte| byte.is_ascii_digit()))
    {
        let room = archive_name[1..3].parse::<i8>().map_err(|_| {
            PlannerContractError::new(
                "return_restart_audit.savmem_placements.placement_room",
                "is outside the signed room domain",
            )
        })?;
        Some(room)
    } else {
        None
    };
    Ok((stage_name.into(), room))
}

fn collect_source_files(
    directory: &Path,
    output: &mut Vec<PathBuf>,
) -> Result<(), PlannerContractError> {
    let mut entries = fs::read_dir(directory)
        .map_err(|error| {
            PlannerContractError::new("return_restart_audit.source_root", error.to_string())
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            PlannerContractError::new("return_restart_audit.source_root", error.to_string())
        })?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            PlannerContractError::new("return_restart_audit.source_file", error.to_string())
        })?;
        if metadata.file_type().is_symlink() {
            return Err(PlannerContractError::new(
                "return_restart_audit.source_file",
                "source traversal rejects symbolic links",
            ));
        }
        if metadata.is_dir() {
            collect_source_files(&path, output)?;
        } else if metadata.is_file()
            && matches!(
                path.extension().and_then(|value| value.to_str()),
                Some("cpp" | "inc")
            )
        {
            if output.len() == MAX_SOURCE_FILES {
                return Err(PlannerContractError::new(
                    "return_restart_audit.source_files",
                    format!("must contain at most {MAX_SOURCE_FILES} source files"),
                ));
            }
            output.push(path);
        }
    }
    Ok(())
}

fn find_call_sites(source: &str) -> Vec<ReturnRestartCallSite> {
    let source = strip_comments(source);
    let mut sites = Vec::new();
    for (line_index, line) in source.lines().enumerate() {
        for (symbol, kind) in WRITER_SYMBOLS {
            let needle = format!("{symbol}(");
            for _ in line.match_indices(&needle) {
                sites.push(ReturnRestartCallSite {
                    line: line_index as u64 + 1,
                    symbol: symbol.into(),
                    writer_kind: kind,
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
    files: &[ReturnRestartSourceFile],
) -> Result<Vec<ReturnRestartWriterCount>, PlannerContractError> {
    let mut counts = BTreeMap::<ReturnRestartWriterKind, (u64, u64)>::new();
    for source in files {
        let mut file_kinds = BTreeMap::<ReturnRestartWriterKind, u64>::new();
        for site in &source.call_sites {
            *file_kinds.entry(site.writer_kind).or_default() += 1;
        }
        for (kind, sites) in file_kinds {
            let row = counts.entry(kind).or_default();
            row.0 = row.0.checked_add(sites).ok_or_else(|| {
                PlannerContractError::new("return_restart_audit.counts", "count overflowed")
            })?;
            row.1 = row.1.checked_add(1).ok_or_else(|| {
                PlannerContractError::new("return_restart_audit.counts", "count overflowed")
            })?;
        }
    }
    Ok(counts
        .into_iter()
        .map(
            |(writer_kind, (call_sites, source_files))| ReturnRestartWriterCount {
                writer_kind,
                call_sites,
                source_files,
            },
        )
        .collect())
}

fn symbol_matches_kind(symbol: &str, kind: ReturnRestartWriterKind) -> bool {
    WRITER_SYMBOLS
        .iter()
        .any(|(expected, expected_kind)| symbol == *expected && kind == *expected_kind)
}

fn relative_path(root: &Path, path: &Path) -> Result<String, PlannerContractError> {
    let relative = path.strip_prefix(root).map_err(|_| {
        PlannerContractError::new(
            "return_restart_audit.source_file",
            "escaped repository root",
        )
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
            "return_restart_audit.relative_path",
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

    #[test]
    fn source_census_ignores_comments_and_keeps_writer_domains_distinct() {
        let sites = find_call_sites(
            "// dComIfGs_setRestartRoom(a,b,c)\nvoid f(){ dComIfGs_setRestartRoom(a,b,c); }\n/* mPlayerReturnPlace.init(); */\nvoid g(){ dComIfGs_setRestartRoomParam(0); }\n",
        );
        assert_eq!(sites.len(), 2);
        assert_eq!(
            sites[0].writer_kind,
            ReturnRestartWriterKind::RestartPlaceSet
        );
        assert_eq!(
            sites[1].writer_kind,
            ReturnRestartWriterKind::RestartRoomParameterSet
        );
    }

    #[test]
    fn bundled_audit_is_canonical_and_reproduces_the_exact_census() {
        let audit = bundled_gz2e01_return_restart_audit().unwrap();
        assert_eq!(audit.source_files.len(), 24);
        assert_eq!(
            audit
                .writer_counts
                .iter()
                .map(|row| row.call_sites)
                .sum::<u64>(),
            30
        );
        assert_eq!(audit.savmem_placements.len(), 132);
        assert_eq!(
            audit.canonical_bytes().unwrap(),
            BUNDLED_GZ2E01_RETURN_RESTART_AUDIT
        );
    }
}
