//! Offline, content-addressed inventory of authored stage resources.
//!
//! The builder reads immutable RARC/DZS/DZR/KCL/PLC bytes. It never links to
//! the game, queries a live process, or treats runtime process IDs as authored
//! identity.

use crate::artifact::Digest;
use crate::world_geometry::{KclInventoryPrism, KclPlc, RarcArchive, Vec3, WorldGeometryError};
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

pub const WORLD_INVENTORY_SCHEMA: &str = "dusklight-world-inventory/v1";
const STAGE_CHUNK_HEADER_SIZE: usize = 12;
const PLACEMENT_SIZE: usize = 0x20;
const SCALED_PLACEMENT_SIZE: usize = 0x24;
const SCLS_SIZE: usize = 0x0d;
const MAX_STAGE_CHUNKS: usize = 4096;
const MAX_STAGE_RECORDS: usize = 1_000_000;

#[derive(Debug)]
pub enum WorldInventoryError {
    Io(std::io::Error),
    Geometry(WorldGeometryError),
    Json(serde_json::Error),
    Invalid(String),
}

impl fmt::Display for WorldInventoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "world inventory I/O error: {error}"),
            Self::Geometry(error) => write!(formatter, "world inventory geometry error: {error}"),
            Self::Json(error) => write!(formatter, "world inventory JSON error: {error}"),
            Self::Invalid(message) => formatter.write_str(message),
        }
    }
}

impl Error for WorldInventoryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Geometry(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::Invalid(_) => None,
        }
    }
}

impl From<std::io::Error> for WorldInventoryError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<WorldGeometryError> for WorldInventoryError {
    fn from(value: WorldGeometryError) -> Self {
        Self::Geometry(value)
    }
}

impl From<serde_json::Error> for WorldInventoryError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Stage,
    Room,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SourceScope {
    pub kind: SourceKind,
    pub room: Option<i8>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorldSource {
    pub scope: SourceScope,
    pub archive_sha256: Digest,
    pub stage_data_path: String,
    pub stage_data_sha256: Digest,
    pub kcl_path: Option<String>,
    pub kcl_sha256: Option<Digest>,
    pub plc_path: Option<String>,
    pub plc_sha256: Option<Digest>,
    pub addressable_prisms: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StageChunkSummary {
    pub source_sha256: Digest,
    pub scope: SourceScope,
    pub tag: String,
    pub record_count: usize,
    pub data_offset: usize,
    pub recognized_record_size: Option<usize>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlacementKind {
    Actor,
    ScaledActor,
    Treasure,
    PlayerSpawn,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlacementRecord {
    pub stable_id: String,
    pub source_sha256: Digest,
    pub scope: SourceScope,
    pub chunk_tag: String,
    pub record_index: usize,
    pub layer: Option<u8>,
    pub kind: PlacementKind,
    pub name: String,
    pub parameters: u32,
    pub position: Vec3,
    pub angle: [i16; 3],
    pub set_id: u16,
    pub scale_raw: Option<[u8; 3]>,
    pub raw_hex: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StageExitRecord {
    pub stable_id: String,
    pub source_sha256: Digest,
    pub scope: SourceScope,
    pub chunk_tag: String,
    pub record_index: usize,
    pub destination_stage: String,
    pub destination_point: i16,
    pub destination_room: i8,
    pub destination_layer: i8,
    pub wipe: u8,
    pub wipe_time: u8,
    pub time_hour: i8,
    pub raw_start: u8,
    pub raw_field_a: u8,
    pub raw_field_b: u8,
    pub raw_wipe: u8,
    pub raw_hex: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CollisionInventoryRecord {
    pub room: i8,
    pub prism: KclInventoryPrism,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CollisionLoadTrigger {
    pub stable_id: String,
    pub room: i8,
    pub collision_id: String,
    pub collision_exit_id: u8,
    pub scls_id: String,
    pub destination_stage: String,
    pub destination_room: i8,
    pub destination_layer: i8,
    pub destination_point: i16,
    pub inferred_semantics: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorldInventory {
    pub schema: String,
    pub stage: String,
    pub sources: Vec<WorldSource>,
    pub chunks: Vec<StageChunkSummary>,
    pub placements: Vec<PlacementRecord>,
    pub player_spawns: Vec<PlacementRecord>,
    pub exits: Vec<StageExitRecord>,
    pub collisions: Vec<CollisionInventoryRecord>,
    pub load_triggers: Vec<CollisionLoadTrigger>,
}

impl WorldInventory {
    pub fn build(stage_dir: &Path, stage: &str) -> Result<Self, WorldInventoryError> {
        validate_stage_name(stage)?;
        let stage_archive_path = stage_dir.join("STG_00.arc");
        if !stage_archive_path.is_file() {
            return Err(WorldInventoryError::Invalid(format!(
                "missing stage archive {}",
                stage_archive_path.display()
            )));
        }

        let mut sources = Vec::new();
        let mut chunks = Vec::new();
        let mut placements = Vec::new();
        let mut player_spawns = Vec::new();
        let mut exits = Vec::new();
        let mut collisions = Vec::new();

        let stage_archive = RarcArchive::parse(&fs::read(&stage_archive_path)?)?;
        let stage_data = stage_archive.unique_basename("stage.dzs")?;
        let stage_data_path = unique_resource_path(&stage_archive, "stage.dzs")?;
        let stage_scope = SourceScope {
            kind: SourceKind::Stage,
            room: None,
        };
        let stage_digest = sha256(stage_data);
        let decoded = decode_stage_data(stage_data, stage_digest, stage_scope, "dzs")?;
        chunks.extend(decoded.chunks);
        placements.extend(decoded.placements);
        player_spawns.extend(decoded.player_spawns);
        exits.extend(decoded.exits);
        sources.push(WorldSource {
            scope: stage_scope,
            archive_sha256: stage_archive.sha256(),
            stage_data_path,
            stage_data_sha256: stage_digest,
            kcl_path: None,
            kcl_sha256: None,
            plc_path: None,
            plc_sha256: None,
            addressable_prisms: 0,
        });

        for (room, path) in room_archives(stage_dir)? {
            let room_scope = SourceScope {
                kind: SourceKind::Room,
                room: Some(room),
            };
            let archive = RarcArchive::parse(&fs::read(path)?)?;
            let dzr = archive.unique_basename("room.dzr")?;
            let dzr_path = unique_resource_path(&archive, "room.dzr")?;
            let dzr_digest = sha256(dzr);
            let decoded = decode_stage_data(dzr, dzr_digest, room_scope, "dzr")?;
            chunks.extend(decoded.chunks);
            placements.extend(decoded.placements);
            player_spawns.extend(decoded.player_spawns);
            exits.extend(decoded.exits);

            let kcl = archive.unique_basename("room.kcl")?;
            let plc = archive.unique_basename("room.plc")?;
            let kcl_path = unique_resource_path(&archive, "room.kcl")?;
            let plc_path = unique_resource_path(&archive, "room.plc")?;
            let parsed = KclPlc::parse(kcl, plc)?;
            for prism_index in 1..parsed.prism_table_count() {
                let prism_index = u16::try_from(prism_index).map_err(|_| {
                    WorldInventoryError::Invalid("KCL prism index exceeds u16".into())
                })?;
                collisions.push(CollisionInventoryRecord {
                    room,
                    prism: parsed.inventory_prism(prism_index)?,
                });
            }
            sources.push(WorldSource {
                scope: room_scope,
                archive_sha256: archive.sha256(),
                stage_data_path: dzr_path,
                stage_data_sha256: dzr_digest,
                kcl_path: Some(kcl_path),
                kcl_sha256: Some(parsed.kcl_sha256()),
                plc_path: Some(plc_path),
                plc_sha256: Some(parsed.plc_sha256()),
                addressable_prisms: parsed.prism_table_count() - 1,
            });
        }

        let load_triggers = join_load_triggers(&collisions, &exits);
        Ok(Self {
            schema: WORLD_INVENTORY_SCHEMA.into(),
            stage: stage.into(),
            sources,
            chunks,
            placements,
            player_spawns,
            exits,
            collisions,
            load_triggers,
        })
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, WorldInventoryError> {
        Ok(serde_json::to_vec(self)?)
    }

    pub fn digest(&self) -> Result<Digest, WorldInventoryError> {
        Ok(sha256(&self.canonical_bytes()?))
    }
}

struct DecodedStageData {
    chunks: Vec<StageChunkSummary>,
    placements: Vec<PlacementRecord>,
    player_spawns: Vec<PlacementRecord>,
    exits: Vec<StageExitRecord>,
}

#[derive(Clone)]
struct ChunkHeader {
    tag: String,
    count: usize,
    offset: usize,
}

fn decode_stage_data(
    bytes: &[u8],
    digest: Digest,
    scope: SourceScope,
    id_prefix: &str,
) -> Result<DecodedStageData, WorldInventoryError> {
    require_range(bytes, 0, 4, "stage-data header")?;
    let chunk_count = read_u32(bytes, 0, "stage-data chunk count")? as usize;
    if chunk_count == 0 || chunk_count > MAX_STAGE_CHUNKS {
        return Err(WorldInventoryError::Invalid(format!(
            "stage-data chunk count {chunk_count} is outside 1..={MAX_STAGE_CHUNKS}"
        )));
    }
    let directory_size = 4_usize
        .checked_add(
            chunk_count
                .checked_mul(STAGE_CHUNK_HEADER_SIZE)
                .ok_or_else(|| WorldInventoryError::Invalid("chunk directory overflow".into()))?,
        )
        .ok_or_else(|| WorldInventoryError::Invalid("chunk directory overflow".into()))?;
    require_range(bytes, 0, directory_size, "stage-data chunk directory")?;
    let mut headers = Vec::with_capacity(chunk_count);
    let mut tags = BTreeSet::new();
    for index in 0..chunk_count {
        let offset = 4 + index * STAGE_CHUNK_HEADER_SIZE;
        let tag = parse_tag(&bytes[offset..offset + 4])?;
        if !tags.insert(tag.clone()) {
            return Err(WorldInventoryError::Invalid(format!(
                "stage-data contains duplicate chunk tag {tag:?}"
            )));
        }
        let count = read_i32(bytes, offset + 4, "stage-data record count")?;
        if count < 0 || count as usize > MAX_STAGE_RECORDS {
            return Err(WorldInventoryError::Invalid(format!(
                "stage-data chunk {tag:?} has invalid record count {count}"
            )));
        }
        let data_offset = read_u32(bytes, offset + 8, "stage-data record offset")? as usize;
        if data_offset < directory_size || data_offset > bytes.len() {
            return Err(WorldInventoryError::Invalid(format!(
                "stage-data chunk {tag:?} starts outside its data area"
            )));
        }
        headers.push(ChunkHeader {
            tag,
            count: count as usize,
            offset: data_offset,
        });
    }

    let mut chunks = Vec::with_capacity(headers.len());
    let mut placements = Vec::new();
    let mut player_spawns = Vec::new();
    let mut exits = Vec::new();
    let mut recognized_ranges = Vec::new();
    for header in headers {
        let record_kind = classify_chunk(&header.tag);
        let record_size = record_kind.map(|kind| kind.record_size());
        chunks.push(StageChunkSummary {
            source_sha256: digest,
            scope,
            tag: header.tag.clone(),
            record_count: header.count,
            data_offset: header.offset,
            recognized_record_size: record_size,
        });
        let Some(kind) = record_kind else {
            continue;
        };
        let byte_count = header
            .count
            .checked_mul(kind.record_size())
            .ok_or_else(|| WorldInventoryError::Invalid("stage record range overflow".into()))?;
        require_range(bytes, header.offset, byte_count, "stage-data records")?;
        let end = header.offset + byte_count;
        recognized_ranges.push((header.offset, end, header.tag.clone()));
        for record_index in 0..header.count {
            let offset = header.offset + record_index * kind.record_size();
            let record = &bytes[offset..offset + kind.record_size()];
            let context = StageRecordContext {
                digest,
                scope,
                id_prefix,
                tag: &header.tag,
                record_index,
            };
            match kind {
                StageRecordKind::Scls => exits.push(parse_scls(record, &context)?),
                StageRecordKind::Player => player_spawns.push(parse_placement(
                    record,
                    &context,
                    PlacementKind::PlayerSpawn,
                    false,
                )?),
                StageRecordKind::Actor(placement_kind, scaled) => {
                    placements.push(parse_placement(record, &context, placement_kind, scaled)?)
                }
            }
        }
    }
    recognized_ranges.sort_by_key(|range| range.0);
    for pair in recognized_ranges.windows(2) {
        if pair[0].1 > pair[1].0 {
            return Err(WorldInventoryError::Invalid(format!(
                "stage-data recognized chunks {:?} and {:?} overlap",
                pair[0].2, pair[1].2
            )));
        }
    }
    Ok(DecodedStageData {
        chunks,
        placements,
        player_spawns,
        exits,
    })
}

#[derive(Clone, Copy)]
enum StageRecordKind {
    Actor(PlacementKind, bool),
    Player,
    Scls,
}

impl StageRecordKind {
    fn record_size(self) -> usize {
        match self {
            Self::Actor(_, false) | Self::Player => PLACEMENT_SIZE,
            Self::Actor(_, true) => SCALED_PLACEMENT_SIZE,
            Self::Scls => SCLS_SIZE,
        }
    }
}

fn classify_chunk(tag: &str) -> Option<StageRecordKind> {
    if tag == "SCLS" {
        return Some(StageRecordKind::Scls);
    }
    if tag == "PLYR" {
        return Some(StageRecordKind::Player);
    }
    if matches!(tag, "ACTR" | "TGOB") || layered_tag(tag, "ACT") {
        return Some(StageRecordKind::Actor(PlacementKind::Actor, false));
    }
    if tag == "TRES" || layered_tag(tag, "TRE") {
        return Some(StageRecordKind::Actor(PlacementKind::Treasure, false));
    }
    if matches!(tag, "SCOB" | "TGSC" | "TGDR" | "Door")
        || layered_tag(tag, "SCO")
        || layered_tag(tag, "Doo")
    {
        return Some(StageRecordKind::Actor(PlacementKind::ScaledActor, true));
    }
    None
}

fn layered_tag(tag: &str, prefix: &str) -> bool {
    tag.len() == 4 && tag.starts_with(prefix) && decode_layer(tag.as_bytes()[3]).is_some()
}

struct StageRecordContext<'a> {
    digest: Digest,
    scope: SourceScope,
    id_prefix: &'a str,
    tag: &'a str,
    record_index: usize,
}

fn parse_placement(
    record: &[u8],
    context: &StageRecordContext<'_>,
    kind: PlacementKind,
    scaled: bool,
) -> Result<PlacementRecord, WorldInventoryError> {
    let name = parse_fixed_name(&record[0..8], "placement name")?;
    let position = Vec3 {
        x: read_f32(record, 12, "placement position X")?,
        y: read_f32(record, 16, "placement position Y")?,
        z: read_f32(record, 20, "placement position Z")?,
    };
    if !position.x.is_finite() || !position.y.is_finite() || !position.z.is_finite() {
        return Err(WorldInventoryError::Invalid(format!(
            "placement {}[{}] has non-finite position",
            context.tag, context.record_index
        )));
    }
    Ok(PlacementRecord {
        stable_id: source_record_id(
            context.id_prefix,
            context.digest,
            context.tag,
            context.record_index,
        ),
        source_sha256: context.digest,
        scope: context.scope,
        chunk_tag: context.tag.into(),
        record_index: context.record_index,
        layer: layer_for_tag(context.tag),
        kind,
        name,
        parameters: read_u32(record, 8, "placement parameters")?,
        position,
        angle: [
            read_i16(record, 24, "placement angle X")?,
            read_i16(record, 26, "placement angle Y")?,
            read_i16(record, 28, "placement angle Z")?,
        ],
        set_id: read_u16(record, 30, "placement set ID")?,
        scale_raw: scaled.then(|| [record[32], record[33], record[34]]),
        raw_hex: hex(record),
    })
}

fn parse_scls(
    record: &[u8],
    context: &StageRecordContext<'_>,
) -> Result<StageExitRecord, WorldInventoryError> {
    let stage = parse_fixed_name(&record[0..8], "SCLS stage name")?;
    let raw_start = record[8];
    let room = record[9] as i8;
    let field_a = record[10];
    let field_b = record[11];
    let raw_wipe = record[12];
    let raw_layer = (field_b & 0x0f) as i8;
    let raw_hour = (((field_a >> 4) & 0x0f) | (field_b & 0x10)) as i8;
    Ok(StageExitRecord {
        stable_id: source_record_id(
            context.id_prefix,
            context.digest,
            context.tag,
            context.record_index,
        ),
        source_sha256: context.digest,
        scope: context.scope,
        chunk_tag: context.tag.into(),
        record_index: context.record_index,
        destination_stage: stage,
        destination_point: i16::from(raw_start),
        destination_room: room,
        destination_layer: if raw_layer >= 15 { -1 } else { raw_layer },
        wipe: if raw_wipe == 15 { 0 } else { raw_wipe },
        wipe_time: (field_b >> 5) & 7,
        time_hour: if raw_hour >= 31 { -1 } else { raw_hour },
        raw_start,
        raw_field_a: field_a,
        raw_field_b: field_b,
        raw_wipe,
        raw_hex: hex(record),
    })
}

fn join_load_triggers(
    collisions: &[CollisionInventoryRecord],
    exits: &[StageExitRecord],
) -> Vec<CollisionLoadTrigger> {
    let mut triggers = Vec::new();
    for collision in collisions {
        let exit_id = collision.prism.authored.code.exit_id;
        if exit_id == 0x3f {
            continue;
        }
        let Some(exit) = exits.iter().find(|exit| {
            exit.scope.room == Some(collision.room)
                && exit.chunk_tag == "SCLS"
                && exit.record_index == usize::from(exit_id)
        }) else {
            continue;
        };
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight.collision-load-trigger/v1\0");
        hasher.update(collision.prism.authored.stable_id.as_bytes());
        hasher.update([0]);
        hasher.update(exit.stable_id.as_bytes());
        let digest = Digest(hasher.finalize().into());
        triggers.push(CollisionLoadTrigger {
            stable_id: format!("load-trigger-sha256:{digest}"),
            room: collision.room,
            collision_id: collision.prism.authored.stable_id.clone(),
            collision_exit_id: exit_id,
            scls_id: exit.stable_id.clone(),
            destination_stage: exit.destination_stage.clone(),
            destination_room: exit.destination_room,
            destination_layer: exit.destination_layer,
            destination_point: exit.destination_point,
            inferred_semantics: true,
        });
    }
    triggers
}

fn room_archives(stage_dir: &Path) -> Result<Vec<(i8, PathBuf)>, WorldInventoryError> {
    let mut rooms = Vec::new();
    for entry in fs::read_dir(stage_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Some(room_digits) = name
            .strip_prefix('R')
            .and_then(|rest| rest.strip_suffix("_00.arc"))
        else {
            continue;
        };
        if room_digits.len() != 2 || !room_digits.bytes().all(|byte| byte.is_ascii_digit()) {
            continue;
        }
        let room: i8 = room_digits.parse().map_err(|_| {
            WorldInventoryError::Invalid(format!("invalid room archive name {name:?}"))
        })?;
        rooms.push((room, entry.path()));
    }
    rooms.sort_by_key(|(room, _)| *room);
    if rooms.is_empty() {
        return Err(WorldInventoryError::Invalid(
            "stage directory contains no R##_00.arc room archives".into(),
        ));
    }
    for pair in rooms.windows(2) {
        if pair[0].0 == pair[1].0 {
            return Err(WorldInventoryError::Invalid(format!(
                "stage directory contains duplicate room archive {}",
                pair[0].0
            )));
        }
    }
    Ok(rooms)
}

fn unique_resource_path(archive: &RarcArchive, name: &str) -> Result<String, WorldInventoryError> {
    let mut matches = archive
        .resources()
        .iter()
        .filter(|entry| entry.name == name);
    let path = matches
        .next()
        .ok_or_else(|| WorldInventoryError::Invalid(format!("missing RARC resource {name:?}")))?
        .path
        .clone();
    if matches.next().is_some() {
        return Err(WorldInventoryError::Invalid(format!(
            "multiple RARC resources named {name:?}"
        )));
    }
    Ok(path)
}

fn validate_stage_name(stage: &str) -> Result<(), WorldInventoryError> {
    if stage.is_empty()
        || stage.len() > 8
        || !stage
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Err(WorldInventoryError::Invalid(
            "stage ID must contain 1..=8 uppercase ASCII letters, digits, or underscore".into(),
        ));
    }
    Ok(())
}

fn source_record_id(prefix: &str, digest: Digest, tag: &str, index: usize) -> String {
    format!("{prefix}-sha256:{digest}/chunk/{tag}/record/{index}")
}

fn layer_for_tag(tag: &str) -> Option<u8> {
    if ["ACT", "SCO", "TRE", "Doo"]
        .iter()
        .any(|prefix| tag.starts_with(prefix))
        && tag.len() == 4
    {
        decode_layer(tag.as_bytes()[3])
    } else {
        None
    }
}

fn decode_layer(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'e' => Some(byte - b'a' + 10),
        _ => None,
    }
}

fn parse_tag(bytes: &[u8]) -> Result<String, WorldInventoryError> {
    if bytes.len() != 4 || !bytes.iter().all(|byte| byte.is_ascii_graphic()) {
        return Err(WorldInventoryError::Invalid(
            "stage-data chunk tag must be four printable ASCII bytes".into(),
        ));
    }
    String::from_utf8(bytes.to_vec())
        .map_err(|_| WorldInventoryError::Invalid("stage-data chunk tag is not UTF-8".into()))
}

fn parse_fixed_name(bytes: &[u8], context: &str) -> Result<String, WorldInventoryError> {
    let length = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    let name = &bytes[..length];
    if name.is_empty() || !name.iter().all(|byte| byte.is_ascii_graphic()) {
        return Err(WorldInventoryError::Invalid(format!(
            "{context} must be nonempty printable ASCII"
        )));
    }
    String::from_utf8(name.to_vec())
        .map_err(|_| WorldInventoryError::Invalid(format!("{context} is not UTF-8")))
}

fn hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}

fn sha256(bytes: &[u8]) -> Digest {
    Digest(Sha256::digest(bytes).into())
}

fn require_range(
    bytes: &[u8],
    offset: usize,
    size: usize,
    context: &str,
) -> Result<(), WorldInventoryError> {
    let end = offset
        .checked_add(size)
        .ok_or_else(|| WorldInventoryError::Invalid(format!("{context} range overflow")))?;
    if end > bytes.len() {
        return Err(WorldInventoryError::Invalid(format!(
            "{context} range {offset:#x}..{end:#x} exceeds file size {:#x}",
            bytes.len()
        )));
    }
    Ok(())
}

fn read_u16(bytes: &[u8], offset: usize, context: &str) -> Result<u16, WorldInventoryError> {
    require_range(bytes, offset, 2, context)?;
    Ok(u16::from_be_bytes(
        bytes[offset..offset + 2].try_into().expect("fixed slice"),
    ))
}

fn read_i16(bytes: &[u8], offset: usize, context: &str) -> Result<i16, WorldInventoryError> {
    require_range(bytes, offset, 2, context)?;
    Ok(i16::from_be_bytes(
        bytes[offset..offset + 2].try_into().expect("fixed slice"),
    ))
}

fn read_u32(bytes: &[u8], offset: usize, context: &str) -> Result<u32, WorldInventoryError> {
    require_range(bytes, offset, 4, context)?;
    Ok(u32::from_be_bytes(
        bytes[offset..offset + 4].try_into().expect("fixed slice"),
    ))
}

fn read_i32(bytes: &[u8], offset: usize, context: &str) -> Result<i32, WorldInventoryError> {
    require_range(bytes, offset, 4, context)?;
    Ok(i32::from_be_bytes(
        bytes[offset..offset + 4].try_into().expect("fixed slice"),
    ))
}

fn read_f32(bytes: &[u8], offset: usize, context: &str) -> Result<f32, WorldInventoryError> {
    Ok(f32::from_bits(read_u32(bytes, offset, context)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world_geometry::KclReconstruction;

    fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
        bytes[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
    }

    fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
    }

    fn put_f32(bytes: &mut [u8], offset: usize, value: f32) {
        put_u32(bytes, offset, value.to_bits());
    }

    fn stage_data() -> Vec<u8> {
        let directory = 4 + 3 * STAGE_CHUNK_HEADER_SIZE;
        let actor_offset = directory;
        let exit_offset = actor_offset + PLACEMENT_SIZE;
        let player_offset = exit_offset + SCLS_SIZE;
        let mut bytes = vec![0; player_offset + PLACEMENT_SIZE];
        put_u32(&mut bytes, 0, 3);
        for (index, (tag, count, offset)) in [
            (b"ACT3", 1_u32, actor_offset),
            (b"SCLS", 1, exit_offset),
            (b"PLYR", 1, player_offset),
        ]
        .into_iter()
        .enumerate()
        {
            let node = 4 + index * STAGE_CHUNK_HEADER_SIZE;
            bytes[node..node + 4].copy_from_slice(tag);
            put_u32(&mut bytes, node + 4, count);
            put_u32(&mut bytes, node + 8, offset as u32);
        }
        bytes[actor_offset..actor_offset + 8].copy_from_slice(b"CamArea\0");
        put_u32(&mut bytes, actor_offset + 8, 0x1234_5678);
        put_f32(&mut bytes, actor_offset + 12, 1.0);
        put_f32(&mut bytes, actor_offset + 16, 2.0);
        put_f32(&mut bytes, actor_offset + 20, 3.0);
        put_u16(&mut bytes, actor_offset + 24, 4);
        put_u16(&mut bytes, actor_offset + 26, 5);
        put_u16(&mut bytes, actor_offset + 28, 6);
        put_u16(&mut bytes, actor_offset + 30, 7);
        bytes[exit_offset..exit_offset + 8].copy_from_slice(b"F_SP104\0");
        bytes[exit_offset + 8] = 0;
        bytes[exit_offset + 9] = 1;
        bytes[exit_offset + 10] = 0xf0;
        bytes[exit_offset + 11] = 0x9f;
        bytes[exit_offset + 12] = 19;
        bytes[player_offset..player_offset + 8].copy_from_slice(b"Link\0\0\0\0");
        bytes
    }

    #[test]
    fn decodes_authored_records_with_structural_ids_and_raw_bytes() {
        let bytes = stage_data();
        let digest = sha256(&bytes);
        let scope = SourceScope {
            kind: SourceKind::Room,
            room: Some(1),
        };
        let decoded = decode_stage_data(&bytes, digest, scope, "dzr").unwrap();
        assert_eq!(decoded.placements.len(), 1);
        let actor = &decoded.placements[0];
        assert_eq!(actor.name, "CamArea");
        assert_eq!(actor.layer, Some(3));
        assert_eq!(actor.parameters, 0x1234_5678);
        assert_eq!(
            actor.position,
            Vec3 {
                x: 1.0,
                y: 2.0,
                z: 3.0
            }
        );
        assert_eq!(actor.angle, [4, 5, 6]);
        assert_eq!(actor.set_id, 7);
        assert_eq!(actor.raw_hex.len(), PLACEMENT_SIZE * 2);
        assert!(actor.stable_id.starts_with("dzr-sha256:"));
        assert_eq!(decoded.player_spawns.len(), 1);
        assert_eq!(decoded.exits.len(), 1);
        let exit = &decoded.exits[0];
        assert_eq!(exit.destination_stage, "F_SP104");
        assert_eq!(exit.destination_room, 1);
        assert_eq!(exit.destination_layer, -1);
        assert_eq!(exit.wipe, 19);
        assert_eq!(exit.wipe_time, 4);
        assert_eq!(exit.time_hour, -1);
    }

    #[test]
    fn rejects_duplicate_chunks_bad_ranges_and_nonfinite_positions() {
        let scope = SourceScope {
            kind: SourceKind::Stage,
            room: None,
        };
        let mut duplicate = stage_data();
        duplicate[4 + STAGE_CHUNK_HEADER_SIZE..4 + STAGE_CHUNK_HEADER_SIZE + 4]
            .copy_from_slice(b"ACT3");
        assert!(decode_stage_data(&duplicate, sha256(&duplicate), scope, "dzs").is_err());

        let mut bad_range = stage_data();
        put_u32(&mut bad_range, 4 + 8, u32::MAX);
        assert!(decode_stage_data(&bad_range, sha256(&bad_range), scope, "dzs").is_err());

        let mut nonfinite = stage_data();
        let actor_offset = 4 + 3 * STAGE_CHUNK_HEADER_SIZE;
        put_u32(&mut nonfinite, actor_offset + 12, f32::NAN.to_bits());
        assert!(decode_stage_data(&nonfinite, sha256(&nonfinite), scope, "dzs").is_err());
    }

    #[test]
    fn stable_ids_change_with_source_content() {
        let first = stage_data();
        let mut second = first.clone();
        second[4 + 3 * STAGE_CHUNK_HEADER_SIZE + 8] ^= 1;
        let scope = SourceScope {
            kind: SourceKind::Room,
            room: Some(1),
        };
        let first_id = decode_stage_data(&first, sha256(&first), scope, "dzr")
            .unwrap()
            .placements[0]
            .stable_id
            .clone();
        let second_id = decode_stage_data(&second, sha256(&second), scope, "dzr")
            .unwrap()
            .placements[0]
            .stable_id
            .clone();
        assert_ne!(first_id, second_id);
    }

    #[test]
    fn real_f_sp103_inventory_matches_content_golden_when_disc_is_present() {
        let stage_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("orig/GZ2E01/files/res/Stage/F_SP103");
        if !stage_dir.is_dir() {
            eprintln!("skipping F_SP103 content golden: original disc data is absent");
            return;
        }

        let inventory = WorldInventory::build(&stage_dir, "F_SP103").unwrap();
        assert_eq!(inventory.sources.len(), 3);
        assert_eq!(inventory.placements.len(), 1_442);
        assert_eq!(inventory.player_spawns.len(), 48);
        assert_eq!(inventory.exits.len(), 44);
        assert_eq!(inventory.collisions.len(), 10_794);
        assert_eq!(inventory.load_triggers.len(), 40);
        assert_eq!(
            inventory.digest().unwrap().to_string(),
            "370675af90d40e5b6d8e17b8dce3ad48873bec74c7f7c05bb69b50de95201e7f"
        );

        let degenerate = inventory
            .collisions
            .iter()
            .filter(|collision| {
                matches!(
                    collision.prism.reconstruction,
                    KclReconstruction::Degenerate { .. }
                )
            })
            .count();
        assert_eq!(degenerate, 4);

        let route_trigger = inventory
            .load_triggers
            .iter()
            .find(|trigger| trigger.room == 1 && trigger.collision_id.ends_with("/prism/2217"))
            .expect("room 1 prism 2217 must resolve to an authored load trigger");
        assert_eq!(route_trigger.collision_exit_id, 1);
        assert_eq!(route_trigger.destination_stage, "F_SP104");
        assert_eq!(route_trigger.destination_room, 1);
        assert_eq!(route_trigger.destination_layer, -1);
        assert_eq!(route_trigger.destination_point, 0);

        assert!(inventory.exits.iter().any(|exit| {
            exit.scope.room == Some(1)
                && exit.record_index == 1
                && exit.destination_stage == "F_SP104"
                && exit.wipe == 19
                && exit.wipe_time == 4
                && exit.time_hour == -1
        }));
    }
}
