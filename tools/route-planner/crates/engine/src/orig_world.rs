//! Direct conversion from planner-native `orig/` extraction into world inventories.
//!
//! The conversion deliberately carries no collision claims: the native bundle
//! does not yet decode KCL/PLC. Sources, chunk directories, placements, player
//! spawns, and SCLS records are complete for every decoded stage archive.

use crate::artifact::Digest;
use crate::orig_discovery::{ExtractedOrigBundle, ExtractedOrigStageArchive};
use crate::orig_extraction::{ExtractedActorPlacement, ExtractedSceneTransition};
use crate::world_data::{
    PlacementKind, PlacementRecord, SourceKind, SourceScope, StageChunkSummary, StageExitRecord,
    Vec3, WORLD_INVENTORY_SCHEMA, WorldInventory,
};
use crate::{PlannerContractError, canonical_json};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const EXTRACTED_ORIG_WORLD_INVENTORIES_SCHEMA: &str =
    "dusklight.route-planner.extracted-orig-world-inventories/v1";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeWorldCoverageStatus {
    Complete,
    Unavailable,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeWorldInventoryCoverage {
    pub chunk_directories: NativeWorldCoverageStatus,
    pub placements: NativeWorldCoverageStatus,
    pub scene_transitions: NativeWorldCoverageStatus,
    pub collision: NativeWorldCoverageStatus,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractedOrigWorldInventories {
    pub schema: String,
    pub content_sha256: Digest,
    pub game_data_sha256: Digest,
    pub source_bundle_sha256: Digest,
    pub coverage: NativeWorldInventoryCoverage,
    pub inventories: Vec<WorldInventory>,
}

impl ExtractedOrigWorldInventories {
    pub fn build(bundle: &ExtractedOrigBundle) -> Result<Self, PlannerContractError> {
        bundle.validate()?;
        let mut by_stage = BTreeMap::<String, Vec<&ExtractedOrigStageArchive>>::new();
        for archive in &bundle.stages {
            let (stage, _) = archive_scope(&archive.relative_path, &archive.resource_name)?;
            by_stage.entry(stage).or_default().push(archive);
        }
        if by_stage.is_empty() || by_stage.len() > 256 {
            return Err(PlannerContractError::new(
                "orig_world.inventories",
                "must contain between 1 and 256 decoded stages",
            ));
        }

        let inventories = by_stage
            .into_iter()
            .map(|(stage, archives)| build_inventory(&stage, archives))
            .collect::<Result<Vec<_>, _>>()?;
        let result = Self {
            schema: EXTRACTED_ORIG_WORLD_INVENTORIES_SCHEMA.into(),
            content_sha256: bundle.content.digest()?,
            game_data_sha256: bundle.content.fingerprint.game_data_sha256,
            source_bundle_sha256: bundle.digest()?,
            coverage: expected_coverage(),
            inventories,
        };
        result.validate()?;
        Ok(result)
    }

    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != EXTRACTED_ORIG_WORLD_INVENTORIES_SCHEMA {
            return Err(PlannerContractError::new(
                "orig_world.schema",
                "is unsupported",
            ));
        }
        if self.content_sha256 == Digest::ZERO
            || self.game_data_sha256 == Digest::ZERO
            || self.source_bundle_sha256 == Digest::ZERO
        {
            return Err(PlannerContractError::new(
                "orig_world.identity",
                "must contain nonzero content, game-data, and source-bundle digests",
            ));
        }
        if self.coverage != expected_coverage() {
            return Err(PlannerContractError::new(
                "orig_world.coverage",
                "v1 requires complete chunk, placement, and SCLS coverage and unavailable collision coverage",
            ));
        }
        if self.inventories.is_empty() || self.inventories.len() > 256 {
            return Err(PlannerContractError::new(
                "orig_world.inventories",
                "must contain between 1 and 256 stages",
            ));
        }
        let mut previous = None;
        for inventory in &self.inventories {
            if previous.is_some_and(|stage: &str| stage >= inventory.stage.as_str()) {
                return Err(PlannerContractError::new(
                    "orig_world.inventories",
                    "must be unique and sorted by stage",
                ));
            }
            validate_native_inventory(inventory)?;
            previous = Some(inventory.stage.as_str());
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PlannerContractError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, PlannerContractError> {
        let inventories: Self = serde_json::from_slice(bytes)?;
        inventories.validate()?;
        if inventories.canonical_bytes()? != bytes {
            return Err(PlannerContractError::new(
                "orig_world",
                "is not canonical JSON",
            ));
        }
        Ok(inventories)
    }

    pub fn digest(&self) -> Result<Digest, PlannerContractError> {
        Ok(Digest(Sha256::digest(self.canonical_bytes()?).into()))
    }
}

fn expected_coverage() -> NativeWorldInventoryCoverage {
    NativeWorldInventoryCoverage {
        chunk_directories: NativeWorldCoverageStatus::Complete,
        placements: NativeWorldCoverageStatus::Complete,
        scene_transitions: NativeWorldCoverageStatus::Complete,
        collision: NativeWorldCoverageStatus::Unavailable,
    }
}

fn build_inventory(
    stage: &str,
    mut archives: Vec<&ExtractedOrigStageArchive>,
) -> Result<WorldInventory, PlannerContractError> {
    archives.sort_by_key(|archive| {
        archive_scope(&archive.relative_path, &archive.resource_name)
            .map(|(_, scope)| scope_order(scope))
            .unwrap_or((2, i16::MAX))
    });
    let mut sources = Vec::new();
    let mut chunks = Vec::new();
    let mut placements = Vec::new();
    let mut player_spawns = Vec::new();
    let mut exits = Vec::new();
    let mut seen_scopes = BTreeSet::new();

    for archive in archives {
        let (archive_stage, scope) = archive_scope(&archive.relative_path, &archive.resource_name)?;
        if archive_stage != stage {
            return Err(PlannerContractError::new(
                "orig_world.archive.stage",
                "does not match its inventory stage",
            ));
        }
        let scope_key = scope_order(scope);
        if !seen_scopes.insert(scope_key) {
            return Err(PlannerContractError::new(
                "orig_world.archive.scope",
                "contains a duplicate stage or room archive",
            ));
        }
        sources.push(WorldSourceBuilder::new(archive, scope).finish());
        chunks.extend(archive.stage.chunks.iter().map(|chunk| StageChunkSummary {
            source_sha256: archive.resource_sha256,
            scope,
            tag: chunk.tag.clone(),
            record_count: chunk.record_count as usize,
            data_offset: chunk.data_offset as usize,
            recognized_record_size: chunk.recognized_record_size.map(usize::from),
        }));

        let mut ordinary = archive
            .stage
            .actor_placements
            .iter()
            .map(|placement| (placement, actor_kind(placement)))
            .chain(
                archive
                    .stage
                    .treasure_placements
                    .iter()
                    .map(|placement| (placement, PlacementKind::Treasure)),
            )
            .collect::<Vec<_>>();
        let chunk_order = archive
            .stage
            .chunks
            .iter()
            .enumerate()
            .map(|(index, chunk)| (chunk.tag.as_str(), index))
            .collect::<BTreeMap<_, _>>();
        ordinary.sort_by_key(|(placement, _)| {
            (
                chunk_order
                    .get(placement.chunk_tag.as_str())
                    .copied()
                    .unwrap_or(usize::MAX),
                placement.record_index,
            )
        });
        for (placement, kind) in ordinary {
            placements.push(convert_placement(
                placement,
                kind,
                archive.resource_sha256,
                scope,
            )?);
        }
        for placement in &archive.stage.player_spawns {
            player_spawns.push(convert_placement(
                placement,
                PlacementKind::PlayerSpawn,
                archive.resource_sha256,
                scope,
            )?);
        }
        for transition in &archive.stage.scene_transitions {
            exits.push(convert_exit(transition, archive.resource_sha256, scope)?);
        }
    }
    if !seen_scopes.contains(&(0, -1)) {
        return Err(PlannerContractError::new(
            "orig_world.sources",
            "is missing STG_00.arc",
        ));
    }
    let inventory = WorldInventory {
        schema: WORLD_INVENTORY_SCHEMA.into(),
        stage: stage.into(),
        sources,
        chunks,
        placements,
        player_spawns,
        exits,
        collisions: Vec::new(),
        load_triggers: Vec::new(),
    };
    validate_native_inventory(&inventory)?;
    Ok(inventory)
}

struct WorldSourceBuilder<'a> {
    archive: &'a ExtractedOrigStageArchive,
    scope: SourceScope,
}

impl<'a> WorldSourceBuilder<'a> {
    fn new(archive: &'a ExtractedOrigStageArchive, scope: SourceScope) -> Self {
        Self { archive, scope }
    }

    fn finish(self) -> crate::world_data::WorldSource {
        crate::world_data::WorldSource {
            scope: self.scope,
            archive_sha256: self.archive.archive_sha256,
            stage_data_path: self.archive.resource_name.clone(),
            stage_data_sha256: self.archive.resource_sha256,
            kcl_path: None,
            kcl_sha256: None,
            plc_path: None,
            plc_sha256: None,
            addressable_prisms: 0,
        }
    }
}

fn archive_scope(
    relative_path: &str,
    resource_name: &str,
) -> Result<(String, SourceScope), PlannerContractError> {
    let parts = relative_path.split('/').collect::<Vec<_>>();
    let ["files", "res", "Stage", stage, file_name] = parts.as_slice() else {
        return Err(PlannerContractError::new(
            "orig_world.archive.relative_path",
            "must be files/res/Stage/STAGE/ARCHIVE.arc",
        ));
    };
    validate_stage_name(stage)?;
    let scope = if *file_name == "STG_00.arc" && resource_name == "stage.dzs" {
        SourceScope {
            kind: SourceKind::Stage,
            room: None,
        }
    } else if resource_name == "room.dzr" {
        let bytes = file_name.as_bytes();
        if bytes.len() != 10
            || bytes[0] != b'R'
            || !bytes[1..3].iter().all(u8::is_ascii_digit)
            || &bytes[3..] != b"_00.arc"
        {
            return Err(PlannerContractError::new(
                "orig_world.archive.relative_path",
                "room archives must use RNN_00.arc",
            ));
        }
        let room = std::str::from_utf8(&bytes[1..3])
            .expect("ASCII digits are UTF-8")
            .parse::<i8>()
            .map_err(|_| PlannerContractError::new("orig_world.archive.room", "is invalid"))?;
        SourceScope {
            kind: SourceKind::Room,
            room: Some(room),
        }
    } else {
        return Err(PlannerContractError::new(
            "orig_world.archive.resource_name",
            "does not match its stage/room archive name",
        ));
    };
    Ok(((*stage).to_owned(), scope))
}

fn scope_order(scope: SourceScope) -> (u8, i16) {
    match scope {
        SourceScope {
            kind: SourceKind::Stage,
            room: None,
        } => (0, -1),
        SourceScope {
            kind: SourceKind::Room,
            room: Some(room),
        } => (1, i16::from(room)),
        _ => (2, i16::MAX),
    }
}

fn actor_kind(placement: &ExtractedActorPlacement) -> PlacementKind {
    if placement.scale_raw.is_some() {
        PlacementKind::ScaledActor
    } else {
        PlacementKind::Actor
    }
}

fn convert_placement(
    placement: &ExtractedActorPlacement,
    kind: PlacementKind,
    source_sha256: Digest,
    scope: SourceScope,
) -> Result<PlacementRecord, PlannerContractError> {
    validate_extracted_placement(placement, kind)?;
    Ok(PlacementRecord {
        stable_id: source_record_id(
            scope,
            source_sha256,
            &placement.chunk_tag,
            placement.record_index as usize,
        ),
        source_sha256,
        scope,
        chunk_tag: placement.chunk_tag.clone(),
        record_index: placement.record_index as usize,
        layer: placement.layer,
        kind,
        name: placement.name.clone(),
        parameters: placement.parameters,
        position: Vec3 {
            x: placement.position[0],
            y: placement.position[1],
            z: placement.position[2],
        },
        angle: placement.angle,
        set_id: placement.set_id,
        scale_raw: placement.scale_raw,
        raw_hex: placement.raw_hex.clone(),
    })
}

fn convert_exit(
    transition: &ExtractedSceneTransition,
    source_sha256: Digest,
    scope: SourceScope,
) -> Result<StageExitRecord, PlannerContractError> {
    let raw = decode_hex_exact(&transition.raw_hex, 13, "orig_world.exit.raw_hex")?;
    let name = fixed_name(&raw[..8], "orig_world.exit.destination_stage")?;
    let raw_layer = raw[11] & 0x0f;
    let raw_hour = ((raw[10] >> 4) & 0x0f) | (raw[11] & 0x10);
    let layer = (raw_layer < 15).then_some(raw_layer);
    let hour = (raw_hour < 31).then_some(raw_hour);
    let wipe_time = (raw[11] >> 5) & 7;
    if name != transition.destination_stage
        || raw[8] != transition.destination_spawn
        || raw[9] as i8 != transition.destination_room
        || layer != transition.scene_layer
        || hour != transition.time_hour
        || raw[12] != transition.wipe
        || wipe_time != transition.wipe_time
    {
        return Err(PlannerContractError::new(
            "orig_world.exit",
            "decoded fields do not match the retained raw SCLS record",
        ));
    }
    let record_index = transition.exit_id as usize;
    Ok(StageExitRecord {
        stable_id: source_record_id(scope, source_sha256, "SCLS", record_index),
        source_sha256,
        scope,
        chunk_tag: "SCLS".into(),
        record_index,
        destination_stage: name,
        destination_point: i16::from(raw[8]),
        destination_room: raw[9] as i8,
        destination_layer: layer.map_or(-1, i8_from_u8),
        wipe: if raw[12] == 15 { 0 } else { raw[12] },
        wipe_time,
        time_hour: hour.map_or(-1, i8_from_u8),
        raw_start: raw[8],
        raw_field_a: raw[10],
        raw_field_b: raw[11],
        raw_wipe: raw[12],
        raw_hex: transition.raw_hex.clone(),
    })
}

fn i8_from_u8(value: u8) -> i8 {
    value as i8
}

fn validate_extracted_placement(
    placement: &ExtractedActorPlacement,
    kind: PlacementKind,
) -> Result<(), PlannerContractError> {
    let scaled = kind == PlacementKind::ScaledActor;
    if (placement.scale_raw.is_some()) != scaled
        || matches!(kind, PlacementKind::Treasure | PlacementKind::PlayerSpawn)
            && placement.scale_raw.is_some()
    {
        return Err(PlannerContractError::new(
            "orig_world.placement.kind",
            "does not match the retained scaled-record fields",
        ));
    }
    let expected_size = if scaled { 36 } else { 32 };
    let raw = decode_hex_exact(
        &placement.raw_hex,
        expected_size,
        "orig_world.placement.raw_hex",
    )?;
    let name = fixed_name(&raw[..8], "orig_world.placement.name")?;
    let parameters = u32::from_be_bytes(raw[8..12].try_into().unwrap());
    let position = [
        f32::from_bits(u32::from_be_bytes(raw[12..16].try_into().unwrap())),
        f32::from_bits(u32::from_be_bytes(raw[16..20].try_into().unwrap())),
        f32::from_bits(u32::from_be_bytes(raw[20..24].try_into().unwrap())),
    ];
    let angle = [
        i16::from_be_bytes(raw[24..26].try_into().unwrap()),
        i16::from_be_bytes(raw[26..28].try_into().unwrap()),
        i16::from_be_bytes(raw[28..30].try_into().unwrap()),
    ];
    let set_id = u16::from_be_bytes(raw[30..32].try_into().unwrap());
    let scale = scaled.then(|| [raw[32], raw[33], raw[34]]);
    if name != placement.name
        || parameters != placement.parameters
        || position.map(f32::to_bits) != placement.position.map(f32::to_bits)
        || angle != placement.angle
        || set_id != placement.set_id
        || scale != placement.scale_raw
        || !position.iter().all(|value| value.is_finite())
        || placement.layer != layer_for_tag(&placement.chunk_tag)
    {
        return Err(PlannerContractError::new(
            "orig_world.placement",
            format!(
                "{} record {} decoded fields do not match the retained raw placement record (name={}, parameters={}, position={}, angle={}, set_id={}, scale={}, layer={})",
                placement.chunk_tag,
                placement.record_index,
                name == placement.name,
                parameters == placement.parameters,
                position.map(f32::to_bits) == placement.position.map(f32::to_bits),
                angle == placement.angle,
                set_id == placement.set_id,
                scale == placement.scale_raw,
                placement.layer == layer_for_tag(&placement.chunk_tag),
            ),
        ));
    }
    Ok(())
}

fn validate_native_inventory(inventory: &WorldInventory) -> Result<(), PlannerContractError> {
    inventory.validate()?;
    if !inventory.collisions.is_empty() || !inventory.load_triggers.is_empty() {
        return Err(PlannerContractError::new(
            "orig_world.collision",
            "must remain empty while collision coverage is unavailable",
        ));
    }
    let mut source_by_digest = BTreeMap::new();
    let mut previous_scope = None;
    for source in &inventory.sources {
        let order = scope_order(source.scope);
        let expected_path = match source.scope {
            SourceScope {
                kind: SourceKind::Stage,
                room: None,
            } => "stage.dzs",
            SourceScope {
                kind: SourceKind::Room,
                room: Some(_),
            } => "room.dzr",
            _ => {
                return Err(PlannerContractError::new(
                    "orig_world.sources.scope",
                    "is not a valid stage or room scope",
                ));
            }
        };
        if previous_scope.is_some_and(|previous| previous >= order)
            || matches!(source.scope.room, Some(room) if room < 0)
            || source.archive_sha256 == Digest::ZERO
            || source.stage_data_sha256 == Digest::ZERO
            || source.stage_data_path != expected_path
            || source.kcl_path.is_some()
            || source.kcl_sha256.is_some()
            || source.plc_path.is_some()
            || source.plc_sha256.is_some()
            || source.addressable_prisms != 0
            || source_by_digest
                .insert(source.stage_data_sha256, source.scope)
                .is_some()
        {
            return Err(PlannerContractError::new(
                "orig_world.sources",
                "must be ordered, unique, content-addressed native DZS/DZR sources without collision claims",
            ));
        }
        previous_scope = Some(order);
    }
    if previous_scope.is_none() || scope_order(inventory.sources[0].scope) != (0, -1) {
        return Err(PlannerContractError::new(
            "orig_world.sources",
            "must begin with one stage source",
        ));
    }

    let mut seen_chunk_keys = BTreeSet::new();
    let chunk_keys = inventory
        .chunks
        .iter()
        .map(|chunk| {
            if source_by_digest.get(&chunk.source_sha256) != Some(&chunk.scope)
                || chunk.tag.len() != 4
                || chunk.record_count > 1_000_000
                || chunk.recognized_record_size != recognized_record_size(&chunk.tag)
                || !seen_chunk_keys.insert((chunk.source_sha256, chunk.tag.as_str()))
            {
                return Err(PlannerContractError::new(
                    "orig_world.chunks",
                    "contains an invalid source, tag, count, or record size",
                ));
            }
            Ok((chunk.source_sha256, chunk.tag.as_str(), chunk.record_count))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut record_ids = BTreeSet::new();
    for placement in inventory.placements.iter().chain(&inventory.player_spawns) {
        let Some((_, _, count)) = chunk_keys.iter().find(|(digest, tag, _)| {
            *digest == placement.source_sha256 && *tag == placement.chunk_tag
        }) else {
            return Err(PlannerContractError::new(
                "orig_world.placements",
                "references an absent chunk",
            ));
        };
        if placement_kind_for_tag(&placement.chunk_tag) != Some(placement.kind)
            || source_by_digest.get(&placement.source_sha256) != Some(&placement.scope)
            || placement.record_index >= *count
            || placement.stable_id
                != source_record_id(
                    placement.scope,
                    placement.source_sha256,
                    &placement.chunk_tag,
                    placement.record_index,
                )
            || !record_ids.insert(placement.stable_id.as_str())
        {
            return Err(PlannerContractError::new(
                "orig_world.placements",
                "contains an invalid, duplicate, or out-of-range source record",
            ));
        }
        let extracted = ExtractedActorPlacement {
            chunk_tag: placement.chunk_tag.clone(),
            record_index: placement.record_index as u32,
            layer: placement.layer,
            name: placement.name.clone(),
            parameters: placement.parameters,
            position: [
                placement.position.x,
                placement.position.y,
                placement.position.z,
            ],
            angle: placement.angle,
            set_id: placement.set_id,
            scale_raw: placement.scale_raw,
            raw_hex: placement.raw_hex.clone(),
        };
        validate_extracted_placement(&extracted, placement.kind)?;
    }
    if inventory
        .placements
        .iter()
        .any(|placement| placement.kind == PlacementKind::PlayerSpawn)
        || inventory
            .player_spawns
            .iter()
            .any(|placement| placement.kind != PlacementKind::PlayerSpawn)
    {
        return Err(PlannerContractError::new(
            "orig_world.placements",
            "must keep ordinary and player-spawn collections distinct",
        ));
    }
    for exit in &inventory.exits {
        let Some((_, _, count)) = chunk_keys
            .iter()
            .find(|(digest, tag, _)| *digest == exit.source_sha256 && *tag == "SCLS")
        else {
            return Err(PlannerContractError::new(
                "orig_world.exits",
                "references an absent SCLS chunk",
            ));
        };
        if source_by_digest.get(&exit.source_sha256) != Some(&exit.scope)
            || exit.record_index >= *count
            || exit.stable_id
                != source_record_id(exit.scope, exit.source_sha256, "SCLS", exit.record_index)
            || !record_ids.insert(exit.stable_id.as_str())
        {
            return Err(PlannerContractError::new(
                "orig_world.exits",
                "contains an invalid, duplicate, or out-of-range SCLS record",
            ));
        }
        let raw = decode_hex_exact(&exit.raw_hex, 13, "orig_world.exit.raw_hex")?;
        let raw_layer = raw[11] & 0x0f;
        let raw_hour = ((raw[10] >> 4) & 0x0f) | (raw[11] & 0x10);
        if fixed_name(&raw[..8], "orig_world.exit.destination_stage")? != exit.destination_stage
            || exit.destination_point != i16::from(raw[8])
            || exit.destination_room != raw[9] as i8
            || exit.destination_layer != if raw_layer < 15 { raw_layer as i8 } else { -1 }
            || exit.wipe != if raw[12] == 15 { 0 } else { raw[12] }
            || exit.wipe_time != (raw[11] >> 5) & 7
            || exit.time_hour != if raw_hour < 31 { raw_hour as i8 } else { -1 }
            || exit.raw_start != raw[8]
            || exit.raw_field_a != raw[10]
            || exit.raw_field_b != raw[11]
            || exit.raw_wipe != raw[12]
        {
            return Err(PlannerContractError::new(
                "orig_world.exits",
                "decoded fields do not match the retained raw SCLS record",
            ));
        }
    }
    for (digest, tag, count) in &chunk_keys {
        if let Some(kind) = placement_kind_for_tag(tag) {
            let records = inventory
                .placements
                .iter()
                .chain(&inventory.player_spawns)
                .filter(|placement| {
                    placement.source_sha256 == *digest
                        && placement.chunk_tag == *tag
                        && placement.kind == kind
                })
                .count();
            if records != *count {
                return Err(PlannerContractError::new(
                    "orig_world.placements",
                    "does not completely cover one recognized placement chunk",
                ));
            }
        } else if *tag == "SCLS"
            && inventory
                .exits
                .iter()
                .filter(|exit| exit.source_sha256 == *digest)
                .count()
                != *count
        {
            return Err(PlannerContractError::new(
                "orig_world.exits",
                "does not completely cover one SCLS chunk",
            ));
        }
    }
    Ok(())
}

fn source_record_id(scope: SourceScope, digest: Digest, tag: &str, record_index: usize) -> String {
    let prefix = match scope.kind {
        SourceKind::Stage => "dzs",
        SourceKind::Room => "dzr",
    };
    format!("{prefix}-sha256:{digest}/chunk/{tag}/record/{record_index}")
}

fn layer_for_tag(tag: &str) -> Option<u8> {
    if matches!(
        tag,
        "ACTR" | "TGOB" | "SCOB" | "TGSC" | "TGDR" | "Door" | "TRES" | "PLYR"
    ) || tag.len() != 4
        || !matches!(&tag[..3], "ACT" | "TRE" | "SCO" | "Doo")
    {
        return None;
    }
    match tag.as_bytes()[3] {
        b'0'..=b'9' => Some(tag.as_bytes()[3] - b'0'),
        b'a'..=b'e' => Some(tag.as_bytes()[3] - b'a' + 10),
        b'A'..=b'E' => Some(tag.as_bytes()[3] - b'A' + 10),
        _ => None,
    }
}

fn placement_kind_for_tag(tag: &str) -> Option<PlacementKind> {
    if tag == "PLYR" {
        return Some(PlacementKind::PlayerSpawn);
    }
    if tag == "TRES" || layered_tag(tag, "TRE") {
        return Some(PlacementKind::Treasure);
    }
    if matches!(tag, "ACTR" | "TGOB") || layered_tag(tag, "ACT") {
        return Some(PlacementKind::Actor);
    }
    if matches!(tag, "SCOB" | "TGSC" | "TGDR" | "Door")
        || layered_tag(tag, "SCO")
        || layered_tag(tag, "Doo")
    {
        return Some(PlacementKind::ScaledActor);
    }
    None
}

fn recognized_record_size(tag: &str) -> Option<usize> {
    if let Some(kind) = placement_kind_for_tag(tag) {
        return Some(if kind == PlacementKind::ScaledActor {
            36
        } else {
            32
        });
    }
    match tag {
        "STAG" => Some(60),
        "SCLS" => Some(13),
        "REVT" => Some(28),
        "LBNK" => Some(3),
        _ => None,
    }
}

fn layered_tag(tag: &str, prefix: &str) -> bool {
    tag.len() == 4 && tag.starts_with(prefix) && layer_for_tag(tag).is_some()
}

fn fixed_name(bytes: &[u8], field: &'static str) -> Result<String, PlannerContractError> {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    if end == 0 || !bytes[..end].iter().all(u8::is_ascii_graphic) {
        return Err(PlannerContractError::new(
            field,
            "must contain printable ASCII before its first NUL",
        ));
    }
    std::str::from_utf8(&bytes[..end])
        .map(str::to_owned)
        .map_err(|_| PlannerContractError::new(field, "must be UTF-8"))
}

fn decode_hex_exact(
    value: &str,
    bytes: usize,
    field: &'static str,
) -> Result<Vec<u8>, PlannerContractError> {
    if value.len() != bytes * 2 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(PlannerContractError::new(
            field,
            format!("must contain exactly {bytes} lowercase hex bytes"),
        ));
    }
    if value.bytes().any(|byte| byte.is_ascii_uppercase()) {
        return Err(PlannerContractError::new(field, "must use lowercase hex"));
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair).expect("hex is ASCII");
            u8::from_str_radix(pair, 16)
                .map_err(|_| PlannerContractError::new(field, "contains invalid hex"))
        })
        .collect()
}

fn validate_stage_name(stage: &str) -> Result<(), PlannerContractError> {
    if stage.is_empty()
        || stage.len() > 8
        || !stage
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(PlannerContractError::new(
            "orig_world.stage",
            "must contain 1-8 ASCII letters, digits, or underscores",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{
        CONTENT_IDENTITY_SCHEMA, ContentFingerprint, ContentIdentity, GamePlatform, GameRegion,
        RUNTIME_CONFIGURATION_SCHEMA, RuntimeConfiguration,
    };
    use crate::orig_extraction::{
        ExtractedStageChunk, ExtractedStageData, extract_unique_rarc_resource, parse_stage_data,
    };
    use crate::world_import::ExtractedWorldFacts;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn digest(byte: u8) -> Digest {
        Digest([byte; 32])
    }

    fn placement_raw(name: &str, parameters: u32, position: [f32; 3]) -> String {
        let mut bytes = [0_u8; 32];
        bytes[..name.len()].copy_from_slice(name.as_bytes());
        bytes[8..12].copy_from_slice(&parameters.to_be_bytes());
        for (index, value) in position.into_iter().enumerate() {
            bytes[12 + index * 4..16 + index * 4].copy_from_slice(&value.to_bits().to_be_bytes());
        }
        bytes[24..26].copy_from_slice(&1_i16.to_be_bytes());
        bytes[26..28].copy_from_slice(&2_i16.to_be_bytes());
        bytes[28..30].copy_from_slice(&3_i16.to_be_bytes());
        bytes[30..32].copy_from_slice(&4_u16.to_be_bytes());
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    fn placement(tag: &str, index: u32, name: &str, parameters: u32) -> ExtractedActorPlacement {
        ExtractedActorPlacement {
            chunk_tag: tag.into(),
            record_index: index,
            layer: layer_for_tag(tag),
            name: name.into(),
            parameters,
            position: [1.0, 2.0, 3.0],
            angle: [1, 2, 3],
            set_id: 4,
            scale_raw: None,
            raw_hex: placement_raw(name, parameters, [1.0, 2.0, 3.0]),
        }
    }

    fn archive(
        relative_path: &str,
        resource_name: &str,
        resource_sha256: Digest,
        stage: ExtractedStageData,
    ) -> ExtractedOrigStageArchive {
        ExtractedOrigStageArchive {
            relative_path: relative_path.into(),
            archive_sha256: digest(resource_sha256.0[0].wrapping_add(1)),
            resource_name: resource_name.into(),
            resource_sha256,
            stage,
        }
    }

    #[test]
    fn converts_native_stage_records_without_collision_claims() {
        let mut scls = [0_u8; 13];
        scls[..6].copy_from_slice(b"F_SP00");
        scls[8] = 7;
        scls[9] = 2;
        scls[10] = 0xf0;
        scls[11] = 0xff;
        scls[12] = 15;
        let scls_hex = scls.iter().map(|byte| format!("{byte:02x}")).collect();
        let stage = archive(
            "files/res/Stage/F_SP00/STG_00.arc",
            "stage.dzs",
            digest(2),
            ExtractedStageData {
                chunks: vec![ExtractedStageChunk {
                    tag: "ACTR".into(),
                    record_count: 1,
                    data_offset: 64,
                    recognized_record_size: Some(32),
                }],
                stage_information: None,
                scene_transitions: Vec::new(),
                map_events: Vec::new(),
                demo_archive_banks: Vec::new(),
                actor_placements: vec![placement("ACTR", 0, "actor", 0x1234)],
                treasure_placements: Vec::new(),
                player_spawns: Vec::new(),
            },
        );
        let room = archive(
            "files/res/Stage/F_SP00/R02_00.arc",
            "room.dzr",
            digest(4),
            ExtractedStageData {
                chunks: vec![
                    ExtractedStageChunk {
                        tag: "PLYR".into(),
                        record_count: 1,
                        data_offset: 64,
                        recognized_record_size: Some(32),
                    },
                    ExtractedStageChunk {
                        tag: "TREa".into(),
                        record_count: 1,
                        data_offset: 96,
                        recognized_record_size: Some(32),
                    },
                    ExtractedStageChunk {
                        tag: "SCLS".into(),
                        record_count: 1,
                        data_offset: 128,
                        recognized_record_size: Some(13),
                    },
                ],
                stage_information: None,
                scene_transitions: vec![ExtractedSceneTransition {
                    exit_id: 0,
                    destination_stage: "F_SP00".into(),
                    destination_spawn: 7,
                    destination_room: 2,
                    scene_layer: None,
                    time_hour: None,
                    wipe: 15,
                    wipe_time: 7,
                    raw_hex: scls_hex,
                }],
                map_events: Vec::new(),
                demo_archive_banks: Vec::new(),
                actor_placements: Vec::new(),
                treasure_placements: vec![placement("TREa", 0, "chest", 9)],
                player_spawns: vec![placement("PLYR", 0, "start", 7)],
            },
        );
        let inventory = build_inventory("F_SP00", vec![&room, &stage]).unwrap();
        assert_eq!(inventory.sources.len(), 2);
        assert_eq!(inventory.sources[0].scope.kind, SourceKind::Stage);
        assert_eq!(inventory.sources[1].scope.room, Some(2));
        assert_eq!(inventory.placements.len(), 2);
        assert_eq!(inventory.placements[1].kind, PlacementKind::Treasure);
        assert_eq!(inventory.player_spawns.len(), 1);
        assert_eq!(inventory.exits[0].raw_field_a, 0xf0);
        assert_eq!(inventory.exits[0].raw_field_b, 0xff);
        assert_eq!(inventory.exits[0].wipe, 0);
        assert!(inventory.collisions.is_empty());
        assert!(inventory.load_triggers.is_empty());
        assert!(inventory.placements[1].stable_id.starts_with("dzr-sha256:"));

        let set = ExtractedOrigWorldInventories {
            schema: EXTRACTED_ORIG_WORLD_INVENTORIES_SCHEMA.into(),
            content_sha256: digest(8),
            game_data_sha256: digest(9),
            source_bundle_sha256: digest(10),
            coverage: expected_coverage(),
            inventories: vec![inventory.clone()],
        };
        let bytes = set.canonical_bytes().unwrap();
        assert_eq!(
            ExtractedOrigWorldInventories::decode_canonical(&bytes).unwrap(),
            set
        );
        let content = ContentIdentity {
            schema: CONTENT_IDENTITY_SCHEMA.into(),
            id: "gcn-us-native-fixture".into(),
            fingerprint: ContentFingerprint {
                platform: GamePlatform::GameCube,
                region: GameRegion::Usa,
                revision: "fixture".into(),
                product_id: "FIXE01".into(),
                executable_sha256: digest(11),
                game_data_sha256: digest(9),
                resource_manifest_sha256: digest(12),
            },
        };
        let runtime = RuntimeConfiguration {
            schema: RUNTIME_CONFIGURATION_SCHEMA.into(),
            content_sha256: content.digest().unwrap(),
            language: "en".into(),
            settings: BTreeMap::new(),
        };
        let mut import_set = set.clone();
        import_set.content_sha256 = content.digest().unwrap();
        let facts =
            ExtractedWorldFacts::build_from_orig_world_inventories(&content, &runtime, &import_set)
                .unwrap();
        assert_eq!(facts.world_context_sha256, None);
        assert_eq!(
            facts.native_inventory_set_sha256,
            Some(import_set.digest().unwrap())
        );
        assert_eq!(facts.static_world_objects.len(), 3);
        assert_eq!(facts.spawns.len(), 1);
        assert_eq!(facts.encoded_exits.len(), 1);
        assert!(
            facts
                .inventories
                .iter()
                .all(|source| source.spatial_index_sha256.is_none())
        );
        let mut mixed_provenance = facts.clone();
        mixed_provenance.world_context_sha256 = Some(digest(13));
        assert!(mixed_provenance.validate().is_err());
        let mut incomplete = set;
        incomplete.inventories[0].player_spawns.clear();
        assert!(incomplete.validate().is_err());
    }

    #[test]
    fn rejects_decoded_fields_that_disagree_with_raw_records() {
        let mut bad = placement("ACTR", 0, "actor", 1);
        bad.parameters = 2;
        assert!(validate_extracted_placement(&bad, PlacementKind::Actor).is_err());
    }

    #[test]
    fn exact_r_sp116_native_inventory_matches_known_record_coverage_when_available() {
        let Some(root) = repository_root() else {
            return;
        };
        let stage_dir = root.join("orig/GZ2E01/files/res/Stage/R_SP116");
        if !stage_dir.is_dir() {
            return;
        }
        let mut archives = Vec::new();
        for (file, resource) in [
            ("STG_00.arc", "stage.dzs"),
            ("R05_00.arc", "room.dzr"),
            ("R06_00.arc", "room.dzr"),
        ] {
            let bytes = fs::read(stage_dir.join(file)).unwrap();
            let resource_bytes = extract_unique_rarc_resource(&bytes, resource).unwrap();
            archives.push(archive(
                &format!("files/res/Stage/R_SP116/{file}"),
                resource,
                Digest(Sha256::digest(&resource_bytes).into()),
                parse_stage_data(&resource_bytes).unwrap(),
            ));
        }
        let refs = archives.iter().collect::<Vec<_>>();
        let inventory = build_inventory("R_SP116", refs).unwrap();
        assert_eq!(inventory.sources.len(), 3);
        assert_eq!(inventory.chunks.len(), 72);
        assert_eq!(inventory.placements.len(), 202);
        assert_eq!(inventory.player_spawns.len(), 14);
        assert_eq!(inventory.exits.len(), 14);
        assert_eq!(
            inventory.sources[2].stage_data_sha256.to_string(),
            "10487ef6754fec1f454c93aa33f605ee9781b4db4b91eed8e864721d76304d40"
        );
    }

    fn repository_root() -> Option<PathBuf> {
        let mut path = Path::new(env!("CARGO_MANIFEST_DIR"));
        for _ in 0..5 {
            if path.join("TASKS.md").is_file() {
                return Some(path.to_path_buf());
            }
            path = path.parent()?;
        }
        None
    }
}
