//! Canonical candidate catalog for read-only stage/spawn observation surveys.

use crate::artifact::Digest;
use crate::world_inventory::{SourceKind, WorldInventory};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::Path;

pub const STAGE_BOOT_CATALOG_SCHEMA: &str = "dusklight-stage-boot-catalog/v1";
const MAX_STAGES: usize = 256;
const MAX_CANDIDATES: usize = 100_000;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BootPointSourceKind {
    RetailPlayerSpawn,
    KnownLoader,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BootPointSource {
    pub kind: BootPointSourceKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stable_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BootLayerSourceKind {
    ResolvedDefault,
    RetailStageChunk,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BootLayerSource {
    pub kind: BootLayerSourceKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk_tag: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StageBootCandidate {
    pub id: String,
    pub stage: String,
    pub room: i8,
    pub point: i16,
    pub layer: i8,
    pub point_sources: Vec<BootPointSource>,
    pub layer_sources: Vec<BootLayerSource>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StageInventoryStatus {
    Complete,
    Unreadable,
    LoaderOnly,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StageCatalogStatus {
    pub stage: String,
    pub resources_present: bool,
    pub inventory_status: StageInventoryStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inventory_sha256: Option<Digest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<String>,
    pub room_count: usize,
    pub player_spawn_count: usize,
    pub candidate_count: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StageBootCatalog {
    pub schema: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub known_loader_sha256: Option<Digest>,
    pub stages: Vec<StageCatalogStatus>,
    pub candidates: Vec<StageBootCandidate>,
}

#[derive(Debug)]
pub enum StageBootCatalogError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Invalid(String),
}

impl fmt::Display for StageBootCatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "stage boot catalog I/O failed: {error}"),
            Self::Json(error) => write!(formatter, "stage boot catalog JSON failed: {error}"),
            Self::Invalid(message) => write!(formatter, "invalid stage boot catalog: {message}"),
        }
    }
}

impl Error for StageBootCatalogError {}

impl From<std::io::Error> for StageBootCatalogError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for StageBootCatalogError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

#[derive(Default)]
struct StageBuilder {
    resources_present: bool,
    inventory_sha256: Option<Digest>,
    inventory_diagnostic: Option<String>,
    rooms: BTreeSet<i8>,
    retail_points: BTreeMap<(i8, i16), BTreeSet<BootPointSource>>,
    global_layers: BTreeMap<i8, BTreeSet<BootLayerSource>>,
    room_layers: BTreeMap<i8, BTreeMap<i8, BTreeSet<BootLayerSource>>>,
    player_spawn_count: usize,
    loader_points: BTreeMap<i8, BTreeSet<i16>>,
}

impl StageBuilder {
    fn absorb_inventory(
        &mut self,
        inventory: &WorldInventory,
    ) -> Result<(), StageBootCatalogError> {
        self.inventory_sha256 = Some(inventory.digest().map_err(|error| {
            StageBootCatalogError::Invalid(format!(
                "cannot identify inventory for {}: {error}",
                inventory.stage
            ))
        })?);
        for source in &inventory.sources {
            if source.scope.kind == SourceKind::Room
                && let Some(room) = source.scope.room
            {
                self.rooms.insert(room);
            }
        }
        for spawn in &inventory.player_spawns {
            let Some(room) = spawn.scope.room else {
                continue;
            };
            self.rooms.insert(room);
            self.player_spawn_count += 1;
            self.retail_points
                .entry((room, spawn.angle[2]))
                .or_default()
                .insert(BootPointSource {
                    kind: BootPointSourceKind::RetailPlayerSpawn,
                    stable_id: Some(spawn.stable_id.clone()),
                });
        }
        for chunk in &inventory.chunks {
            let Some(layer) = layer_from_chunk_tag(&chunk.tag) else {
                continue;
            };
            let source = BootLayerSource {
                kind: BootLayerSourceKind::RetailStageChunk,
                chunk_tag: Some(chunk.tag.clone()),
            };
            if let Some(room) = chunk.scope.room {
                self.room_layers
                    .entry(room)
                    .or_default()
                    .entry(layer)
                    .or_default()
                    .insert(source);
            } else {
                self.global_layers.entry(layer).or_default().insert(source);
            }
        }
        Ok(())
    }
}

impl StageBootCatalog {
    pub fn build(
        stage_root: &Path,
        known_loader_path: Option<&Path>,
    ) -> Result<Self, StageBootCatalogError> {
        let loader_bytes = known_loader_path.map(fs::read).transpose()?;
        let loader_points = loader_bytes
            .as_deref()
            .map(parse_known_loader_points)
            .unwrap_or_default();
        let mut builders = BTreeMap::<String, StageBuilder>::new();

        for entry in fs::read_dir(stage_root)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let Some(stage) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if !valid_stage_name(&stage) {
                continue;
            }
            let builder = builders.entry(stage.clone()).or_default();
            builder.resources_present = true;
            match WorldInventory::build(&entry.path(), &stage) {
                Ok(inventory) => builder.absorb_inventory(&inventory)?,
                Err(error) => {
                    builder.inventory_diagnostic = Some(stable_inventory_diagnostic(
                        &error.to_string(),
                        &entry.path(),
                    ));
                }
            }
        }

        for ((stage, room), points) in loader_points {
            builders
                .entry(stage)
                .or_default()
                .loader_points
                .entry(room)
                .or_default()
                .extend(points);
        }

        let catalog = Self::from_builders(
            builders,
            loader_bytes
                .as_deref()
                .map(|bytes| Digest(Sha256::digest(bytes).into())),
        )?;
        catalog.validate()?;
        Ok(catalog)
    }

    fn from_builders(
        builders: BTreeMap<String, StageBuilder>,
        known_loader_sha256: Option<Digest>,
    ) -> Result<Self, StageBootCatalogError> {
        let mut stages = Vec::with_capacity(builders.len());
        let mut candidates = Vec::new();

        for (stage, builder) in builders {
            let mut rooms = builder.rooms;
            let mut points = builder.retail_points;

            for (room, loader_points) in &builder.loader_points {
                rooms.insert(*room);
                for point in loader_points {
                    points
                        .entry((*room, *point))
                        .or_default()
                        .insert(BootPointSource {
                            kind: BootPointSourceKind::KnownLoader,
                            stable_id: None,
                        });
                }
            }

            let candidate_start = candidates.len();
            for ((room, point), point_sources) in points {
                let mut layers = BTreeMap::<i8, BTreeSet<BootLayerSource>>::new();
                layers.entry(-1).or_default().insert(BootLayerSource {
                    kind: BootLayerSourceKind::ResolvedDefault,
                    chunk_tag: None,
                });
                for (layer, sources) in &builder.global_layers {
                    layers.entry(*layer).or_default().extend(sources.clone());
                }
                if let Some(specific) = builder.room_layers.get(&room) {
                    for (layer, sources) in specific {
                        layers.entry(*layer).or_default().extend(sources.clone());
                    }
                }
                for (layer, layer_sources) in layers {
                    let id = format!("{stage}/room/{room}/point/{point}/layer/{layer}");
                    candidates.push(StageBootCandidate {
                        id,
                        stage: stage.clone(),
                        room,
                        point,
                        layer,
                        point_sources: point_sources.iter().cloned().collect(),
                        layer_sources: layer_sources.into_iter().collect(),
                    });
                }
            }

            let inventory_status = if builder.inventory_sha256.is_some() {
                StageInventoryStatus::Complete
            } else if builder.resources_present {
                StageInventoryStatus::Unreadable
            } else {
                StageInventoryStatus::LoaderOnly
            };
            stages.push(StageCatalogStatus {
                stage,
                resources_present: builder.resources_present,
                inventory_status,
                inventory_sha256: builder.inventory_sha256,
                diagnostic: builder.inventory_diagnostic,
                room_count: rooms.len(),
                player_spawn_count: builder.player_spawn_count,
                candidate_count: candidates.len() - candidate_start,
            });
        }

        Ok(Self {
            schema: STAGE_BOOT_CATALOG_SCHEMA.into(),
            known_loader_sha256,
            stages,
            candidates,
        })
    }

    pub fn validate(&self) -> Result<(), StageBootCatalogError> {
        if self.schema != STAGE_BOOT_CATALOG_SCHEMA {
            return Err(StageBootCatalogError::Invalid("unsupported schema".into()));
        }
        if self.stages.is_empty() || self.stages.len() > MAX_STAGES {
            return Err(StageBootCatalogError::Invalid(
                "catalog must contain between 1 and 256 stages".into(),
            ));
        }
        if self.candidates.len() > MAX_CANDIDATES {
            return Err(StageBootCatalogError::Invalid(
                "catalog exceeds 100,000 boot candidates".into(),
            ));
        }

        let mut previous_stage = None;
        let mut candidate_counts = BTreeMap::<&str, usize>::new();
        for stage in &self.stages {
            if !valid_stage_name(&stage.stage)
                || previous_stage.is_some_and(|previous: &str| previous >= stage.stage.as_str())
            {
                return Err(StageBootCatalogError::Invalid(
                    "stages must be unique and sorted".into(),
                ));
            }
            previous_stage = Some(&stage.stage);
            match stage.inventory_status {
                StageInventoryStatus::Complete
                    if !stage.resources_present
                        || stage.inventory_sha256.is_none()
                        || stage.diagnostic.is_some() =>
                {
                    return Err(StageBootCatalogError::Invalid(
                        "complete stage has inconsistent inventory evidence".into(),
                    ));
                }
                StageInventoryStatus::Unreadable
                    if !stage.resources_present
                        || stage.inventory_sha256.is_some()
                        || stage.diagnostic.as_deref().is_none_or(str::is_empty) =>
                {
                    return Err(StageBootCatalogError::Invalid(
                        "unreadable stage has inconsistent inventory evidence".into(),
                    ));
                }
                StageInventoryStatus::LoaderOnly
                    if stage.resources_present
                        || stage.inventory_sha256.is_some()
                        || stage.diagnostic.is_some() =>
                {
                    return Err(StageBootCatalogError::Invalid(
                        "loader-only stage has inconsistent inventory evidence".into(),
                    ));
                }
                _ => {}
            }
        }

        let known_stages = self
            .stages
            .iter()
            .map(|stage| stage.stage.as_str())
            .collect::<BTreeSet<_>>();
        let mut previous_candidate = None;
        for candidate in &self.candidates {
            let key = (
                candidate.stage.as_str(),
                candidate.room,
                candidate.point,
                candidate.layer,
            );
            if !known_stages.contains(candidate.stage.as_str())
                || previous_candidate.is_some_and(|previous| previous >= key)
                || candidate.id
                    != format!(
                        "{}/room/{}/point/{}/layer/{}",
                        candidate.stage, candidate.room, candidate.point, candidate.layer
                    )
                || candidate.point_sources.is_empty()
                || candidate.layer_sources.is_empty()
                || !strictly_sorted(&candidate.point_sources)
                || !strictly_sorted(&candidate.layer_sources)
            {
                return Err(StageBootCatalogError::Invalid(
                    "boot candidates are invalid, duplicate, or noncanonical".into(),
                ));
            }
            if candidate
                .point_sources
                .iter()
                .any(|source| match source.kind {
                    BootPointSourceKind::RetailPlayerSpawn => {
                        source.stable_id.as_deref().is_none_or(str::is_empty)
                    }
                    BootPointSourceKind::KnownLoader => source.stable_id.is_some(),
                })
                || candidate
                    .layer_sources
                    .iter()
                    .any(|source| match source.kind {
                        BootLayerSourceKind::ResolvedDefault => {
                            source.chunk_tag.is_some() || candidate.layer != -1
                        }
                        BootLayerSourceKind::RetailStageChunk => source
                            .chunk_tag
                            .as_deref()
                            .is_none_or(|tag| layer_from_chunk_tag(tag) != Some(candidate.layer)),
                    })
            {
                return Err(StageBootCatalogError::Invalid(
                    "boot candidate source evidence is invalid".into(),
                ));
            }
            if candidate
                .point_sources
                .iter()
                .any(|source| source.kind == BootPointSourceKind::KnownLoader)
                && self.known_loader_sha256.is_none()
            {
                return Err(StageBootCatalogError::Invalid(
                    "known-loader candidate lacks loader identity".into(),
                ));
            }
            *candidate_counts
                .entry(candidate.stage.as_str())
                .or_default() += 1;
            previous_candidate = Some(key);
        }
        for stage in &self.stages {
            if stage.candidate_count
                != candidate_counts
                    .get(stage.stage.as_str())
                    .copied()
                    .unwrap_or(0)
            {
                return Err(StageBootCatalogError::Invalid(format!(
                    "candidate count disagrees for {}",
                    stage.stage
                )));
            }
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, StageBootCatalogError> {
        self.validate()?;
        let mut bytes = serde_json::to_vec(self)?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, StageBootCatalogError> {
        let catalog: Self = serde_json::from_slice(bytes)?;
        catalog.validate()?;
        if catalog.canonical_bytes()? != bytes {
            return Err(StageBootCatalogError::Invalid(
                "catalog is not canonical JSON".into(),
            ));
        }
        Ok(catalog)
    }

    pub fn digest(&self) -> Result<Digest, StageBootCatalogError> {
        Ok(Digest(Sha256::digest(self.canonical_bytes()?).into()))
    }
}

fn strictly_sorted<T: Ord>(values: &[T]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
}

fn valid_stage_name(stage: &str) -> bool {
    !stage.is_empty()
        && stage.len() <= 8
        && stage
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
}

fn stable_inventory_diagnostic(message: &str, stage_dir: &Path) -> String {
    message.replace(&stage_dir.display().to_string(), "<stage-dir>")
}

fn layer_from_chunk_tag(tag: &str) -> Option<i8> {
    if tag.len() != 4
        || !["ACT", "SCO", "TRE", "Doo"]
            .iter()
            .any(|prefix| tag.starts_with(prefix))
    {
        return None;
    }
    match tag.as_bytes()[3] {
        b'0'..=b'9' => Some((tag.as_bytes()[3] - b'0') as i8),
        b'a'..=b'e' => Some((tag.as_bytes()[3] - b'a' + 10) as i8),
        _ => None,
    }
}

fn parse_known_loader_points(source: &[u8]) -> BTreeMap<(String, i8), BTreeSet<i16>> {
    let Ok(source) = std::str::from_utf8(source) else {
        return BTreeMap::new();
    };
    let mut result = BTreeMap::<(String, i8), BTreeSet<i16>>::new();
    let mut cursor = 0;
    while let Some(offset) = source[cursor..].find("MapEntry(") {
        let open = cursor + offset + "MapEntry".len();
        let Some(close) = matching_delimiter(source, open, b'(', b')') else {
            break;
        };
        let entry = &source[open + 1..close];
        let strings = quoted_ranges(entry);
        if let Some(stage_range) = strings.get(1) {
            let stage = &entry[stage_range.clone()];
            if valid_stage_name(stage)
                && let Some(room_offset) = entry[stage_range.end + 1..].find('{')
            {
                let room_open = stage_range.end + 1 + room_offset;
                if let Some(room_close) = matching_delimiter(entry, room_open, b'{', b'}') {
                    parse_room_initializer(&entry[room_open..=room_close], stage, &mut result);
                }
            }
        }
        cursor = close + 1;
    }
    result
}

fn quoted_ranges(source: &str) -> Vec<std::ops::Range<usize>> {
    let bytes = source.as_bytes();
    let mut ranges = Vec::new();
    let mut cursor = 0;
    while cursor < bytes.len() {
        if bytes[cursor] != b'"' {
            cursor += 1;
            continue;
        }
        let start = cursor + 1;
        cursor += 1;
        while cursor < bytes.len() && bytes[cursor] != b'"' {
            cursor += if bytes[cursor] == b'\\' { 2 } else { 1 };
        }
        if cursor <= bytes.len() {
            ranges.push(start..cursor);
        }
        cursor += 1;
    }
    ranges
}

fn matching_delimiter(source: &str, open: usize, left: u8, right: u8) -> Option<usize> {
    let bytes = source.as_bytes();
    if bytes.get(open) != Some(&left) {
        return None;
    }
    let mut depth = 0;
    let mut quoted = false;
    let mut cursor = open;
    while cursor < bytes.len() {
        let byte = bytes[cursor];
        if quoted {
            if byte == b'\\' {
                cursor += 2;
                continue;
            }
            quoted = byte != b'"';
        } else if byte == b'"' {
            quoted = true;
        } else if byte == left {
            depth += 1;
        } else if byte == right {
            depth -= 1;
            if depth == 0 {
                return Some(cursor);
            }
        }
        cursor += 1;
    }
    None
}

fn parse_room_initializer(
    source: &str,
    stage: &str,
    result: &mut BTreeMap<(String, i8), BTreeSet<i16>>,
) {
    let bytes = source.as_bytes();
    let mut depth = 0;
    let mut room_start = None;
    for (index, byte) in bytes.iter().copied().enumerate() {
        if byte == b'{' {
            depth += 1;
            if depth == 2 {
                room_start = Some(index);
            }
        } else if byte == b'}' {
            if depth == 2
                && let Some(start) = room_start.take()
            {
                let values = signed_integers(&source[start..=index]);
                if let Some((&room, points)) = values.split_first()
                    && let Ok(room) = i8::try_from(room)
                {
                    result
                        .entry((stage.to_owned(), room))
                        .or_default()
                        .extend(points.iter().filter_map(|point| i16::try_from(*point).ok()));
                }
            }
            depth -= 1;
        }
    }
}

fn signed_integers(source: &str) -> Vec<i32> {
    source
        .split(|character: char| !character.is_ascii_digit() && character != '-')
        .filter(|token| !token.is_empty() && *token != "-")
        .filter_map(|token| token.parse().ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world_geometry::Vec3;
    use crate::world_inventory::{
        PlacementKind, PlacementRecord, SourceScope, StageChunkSummary, WORLD_INVENTORY_SCHEMA,
        WorldSource,
    };

    fn digest(byte: u8) -> Digest {
        Digest([byte; 32])
    }

    fn inventory() -> WorldInventory {
        let room_scope = SourceScope {
            kind: SourceKind::Room,
            room: Some(1),
        };
        WorldInventory {
            schema: WORLD_INVENTORY_SCHEMA.into(),
            stage: "F_SP103".into(),
            sources: vec![
                WorldSource {
                    scope: SourceScope {
                        kind: SourceKind::Stage,
                        room: None,
                    },
                    archive_sha256: digest(1),
                    stage_data_path: "stage.dzs".into(),
                    stage_data_sha256: digest(2),
                    kcl_path: None,
                    kcl_sha256: None,
                    plc_path: None,
                    plc_sha256: None,
                    addressable_prisms: 0,
                },
                WorldSource {
                    scope: room_scope,
                    archive_sha256: digest(3),
                    stage_data_path: "room.dzr".into(),
                    stage_data_sha256: digest(4),
                    kcl_path: Some("room.kcl".into()),
                    kcl_sha256: Some(digest(5)),
                    plc_path: Some("room.plc".into()),
                    plc_sha256: Some(digest(6)),
                    addressable_prisms: 0,
                },
            ],
            chunks: vec![StageChunkSummary {
                source_sha256: digest(4),
                scope: room_scope,
                tag: "ACT3".into(),
                record_count: 0,
                data_offset: 0,
                recognized_record_size: Some(32),
            }],
            placements: Vec::new(),
            player_spawns: vec![PlacementRecord {
                stable_id: "retail-spawn-7".into(),
                source_sha256: digest(4),
                scope: room_scope,
                chunk_tag: "PLYR".into(),
                record_index: 0,
                layer: None,
                kind: PlacementKind::PlayerSpawn,
                name: "Link".into(),
                parameters: 0,
                position: Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                angle: [0, 0, 7],
                set_id: 0,
                scale_raw: None,
                raw_hex: "00".repeat(32),
            }],
            exits: Vec::new(),
            collisions: Vec::new(),
            load_triggers: Vec::new(),
        }
    }

    fn catalog_from(inventory: WorldInventory, loader: &[u8]) -> StageBootCatalog {
        let mut builders = BTreeMap::new();
        let stage = inventory.stage.clone();
        builders.insert(
            stage.clone(),
            StageBuilder {
                resources_present: true,
                loader_points: parse_known_loader_points(loader)
                    .remove(&("F_SP103".into(), 1))
                    .map(|points| BTreeMap::from([(1, points)]))
                    .unwrap_or_default(),
                ..StageBuilder::default()
            },
        );
        builders
            .get_mut(&stage)
            .unwrap()
            .absorb_inventory(&inventory)
            .unwrap();
        StageBootCatalog::from_builders(builders, Some(Digest(Sha256::digest(loader).into())))
            .unwrap()
    }

    #[test]
    fn catalog_merges_retail_and_loader_points_and_crosses_declared_layers() {
        let loader = br#"MapEntry("Ordon", "F_SP103", {{1, {7, 9}}})"#;
        let catalog = catalog_from(inventory(), loader);
        catalog.validate().unwrap();
        assert_eq!(catalog.stages[0].player_spawn_count, 1);
        assert_eq!(catalog.stages[0].candidate_count, 4);
        assert_eq!(
            catalog
                .candidates
                .iter()
                .map(|entry| (entry.point, entry.layer))
                .collect::<Vec<_>>(),
            [(7, -1), (7, 3), (9, -1), (9, 3)]
        );
        assert_eq!(catalog.candidates[0].point_sources.len(), 2);
        assert_eq!(catalog.candidates[2].point_sources.len(), 1);
        let bytes = catalog.canonical_bytes().unwrap();
        assert_eq!(StageBootCatalog::decode_canonical(&bytes).unwrap(), catalog);
        assert_ne!(catalog.digest().unwrap(), Digest::ZERO);
    }

    #[test]
    fn parser_collects_duplicate_map_entries_without_duplicate_points() {
        let loader = br#"
            MapEntry("Village", "F_SP103", {{0, {0, 1, 1}}, {1, {2}}}),
            MapEntry("House", "R_SP01", {{4, {0, 3}}}),
            MapEntry("Village alias", "F_SP103", {{0, {2}}})
        "#;
        let points = parse_known_loader_points(loader);
        assert_eq!(points[&("F_SP103".into(), 0)], BTreeSet::from([0, 1, 2]));
        assert_eq!(points[&("F_SP103".into(), 1)], BTreeSet::from([2]));
        assert_eq!(points[&("R_SP01".into(), 4)], BTreeSet::from([0, 3]));
    }

    #[test]
    fn validation_rejects_noncanonical_candidate_order() {
        let loader = br#"MapEntry("Ordon", "F_SP103", {{1, {7, 9}}})"#;
        let mut catalog = catalog_from(inventory(), loader);
        catalog.candidates.swap(0, 1);
        assert!(catalog.validate().is_err());
    }

    #[test]
    #[ignore = "requires the user's extracted retail disc resources"]
    fn extracted_retail_catalog_is_canonical_and_machine_readable() {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../..")
            .canonicalize()
            .unwrap();
        let catalog = StageBootCatalog::build(
            &repository.join("orig/GZ2E01/files/res/Stage"),
            Some(&repository.join("include/dusk/map_loader_definitions.h")),
        )
        .unwrap();
        let bytes = catalog.canonical_bytes().unwrap();
        assert_eq!(StageBootCatalog::decode_canonical(&bytes).unwrap(), catalog);
        assert!(
            catalog
                .stages
                .iter()
                .any(|stage| stage.inventory_status == StageInventoryStatus::Complete)
        );
        assert!(!catalog.candidates.is_empty());

        let output = repository.join("build/stage-survey/boot-catalog.json");
        fs::create_dir_all(output.parent().unwrap()).unwrap();
        fs::write(&output, bytes).unwrap();
        eprintln!(
            "wrote {} stages and {} candidates to {}",
            catalog.stages.len(),
            catalog.candidates.len(),
            output.display()
        );
    }
}
