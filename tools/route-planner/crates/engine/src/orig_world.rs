//! Direct conversion from planner-native `orig/` extraction into world inventories.
//!
//! The conversion deliberately carries no collision claims: the native bundle
//! does not yet decode KCL/PLC. Sources, chunk directories, placements, player
//! spawns, and SCLS records are complete for every decoded stage archive.

use crate::artifact::Digest;
use crate::orig_discovery::{ExtractedOrigBundle, ExtractedOrigStageArchive};
use crate::orig_extraction::{
    ExtractedActorPlacement, ExtractedCamera, ExtractedCameraArrow, ExtractedFileList,
    ExtractedPath, ExtractedPathPoint, ExtractedRoomRead, ExtractedRoomTransform,
    ExtractedSceneTransition,
};
use crate::world_data::{
    PlacementKind, PlacementRecord, SourceKind, SourceScope, StageChunkSummary, StageExitRecord,
    Vec3, WORLD_INVENTORY_SCHEMA, WorldInventory,
};
use crate::{PlannerContractError, canonical_json};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};

mod validation;
use validation::*;

pub const EXTRACTED_ORIG_WORLD_INVENTORIES_SCHEMA: &str =
    "dusklight.route-planner.extracted-orig-world-inventories/v5";

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
    pub map_room_metadata: NativeWorldCoverageStatus,
    pub collision: NativeWorldCoverageStatus,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeRoomTransformRecord {
    pub stage: String,
    pub source_sha256: Digest,
    pub scope: SourceScope,
    pub transform: ExtractedRoomTransform,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeFileListRecord {
    pub stage: String,
    pub source_sha256: Digest,
    pub scope: SourceScope,
    pub file_list: ExtractedFileList,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeRoomReadRecord {
    pub stage: String,
    pub source_sha256: Digest,
    pub scope: SourceScope,
    pub room_read: ExtractedRoomRead,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeCameraRecord {
    pub stage: String,
    pub source_sha256: Digest,
    pub scope: SourceScope,
    pub camera: ExtractedCamera,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeCameraArrowRecord {
    pub stage: String,
    pub source_sha256: Digest,
    pub scope: SourceScope,
    pub arrow: ExtractedCameraArrow,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativePathRecord {
    pub stage: String,
    pub source_sha256: Digest,
    pub scope: SourceScope,
    pub path: ExtractedPath,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativePathPointRecord {
    pub stage: String,
    pub source_sha256: Digest,
    pub scope: SourceScope,
    pub point: ExtractedPathPoint,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeStageMetadata {
    pub stage: String,
    pub room_transforms: Vec<NativeRoomTransformRecord>,
    pub file_lists: Vec<NativeFileListRecord>,
    pub room_reads: Vec<NativeRoomReadRecord>,
    pub cameras: Vec<NativeCameraRecord>,
    pub camera_arrows: Vec<NativeCameraArrowRecord>,
    pub paths: Vec<NativePathRecord>,
    pub path_points: Vec<NativePathPointRecord>,
}

impl NativeStageMetadata {
    pub(crate) fn validate_records(&self) -> Result<(), PlannerContractError> {
        validate_stage_name(&self.stage)?;
        let mut transform_keys = BTreeSet::new();
        let mut previous_transform = None;
        for record in &self.room_transforms {
            let order = (scope_order(record.scope), record.transform.record_index);
            if record.stage != self.stage
                || record.source_sha256 == Digest::ZERO
                || record.scope
                    != (SourceScope {
                        kind: SourceKind::Stage,
                        room: None,
                    })
                || !transform_keys.insert((record.source_sha256, record.transform.record_index))
                || previous_transform.is_some_and(|previous| previous >= order)
            {
                return Err(PlannerContractError::new(
                    "orig_world.room_transforms",
                    "must be ordered unique stage-scope source records",
                ));
            }
            validate_room_transform_raw(&record.transform)?;
            previous_transform = Some(order);
        }
        let mut file_list_keys = BTreeSet::new();
        let mut previous_file_list = None;
        for record in &self.file_lists {
            let order = (scope_order(record.scope), record.file_list.record_index);
            if record.stage != self.stage
                || record.source_sha256 == Digest::ZERO
                || !matches!(
                    record.scope,
                    SourceScope {
                        kind: SourceKind::Stage,
                        room: None
                    } | SourceScope {
                        kind: SourceKind::Room,
                        room: Some(0..=i8::MAX)
                    }
                )
                || !file_list_keys.insert((record.source_sha256, record.file_list.record_index))
                || previous_file_list.is_some_and(|previous| previous >= order)
            {
                return Err(PlannerContractError::new(
                    "orig_world.file_lists",
                    "must be ordered unique records with valid stage or room scope",
                ));
            }
            validate_file_list_raw(&record.file_list)?;
            previous_file_list = Some(order);
        }
        let mut room_read_keys = BTreeSet::new();
        let mut previous_room_read = None;
        for record in &self.room_reads {
            let order = (scope_order(record.scope), record.room_read.room_index);
            if record.stage != self.stage
                || record.source_sha256 == Digest::ZERO
                || !matches!(
                    record.scope,
                    SourceScope {
                        kind: SourceKind::Stage,
                        room: None
                    } | SourceScope {
                        kind: SourceKind::Room,
                        room: Some(0..=i8::MAX)
                    }
                )
                || !room_read_keys.insert((record.source_sha256, record.room_read.room_index))
                || previous_room_read.is_some_and(|previous| previous >= order)
            {
                return Err(PlannerContractError::new(
                    "orig_world.room_reads",
                    "must be ordered unique records with valid stage or room scope",
                ));
            }
            validate_room_read_raw(&record.room_read)?;
            previous_room_read = Some(order);
        }
        let mut camera_keys = BTreeSet::new();
        let mut previous_camera = None;
        for record in &self.cameras {
            let order = (scope_order(record.scope), record.camera.record_index);
            if record.stage != self.stage
                || record.source_sha256 == Digest::ZERO
                || !valid_stage_or_room_scope(record.scope)
                || !camera_keys.insert((record.source_sha256, record.camera.record_index))
                || previous_camera.is_some_and(|previous| previous >= order)
            {
                return Err(PlannerContractError::new(
                    "orig_world.cameras",
                    "must be ordered unique records with valid stage or room scope",
                ));
            }
            validate_camera_raw(&record.camera)?;
            previous_camera = Some(order);
        }
        let mut arrow_keys = BTreeSet::new();
        let mut previous_arrow = None;
        for record in &self.camera_arrows {
            let order = (scope_order(record.scope), record.arrow.record_index);
            if record.stage != self.stage
                || record.source_sha256 == Digest::ZERO
                || !valid_stage_or_room_scope(record.scope)
                || !arrow_keys.insert((record.source_sha256, record.arrow.record_index))
                || previous_arrow.is_some_and(|previous| previous >= order)
            {
                return Err(PlannerContractError::new(
                    "orig_world.camera_arrows",
                    "must be ordered unique records with valid stage or room scope",
                ));
            }
            validate_camera_arrow_raw(&record.arrow)?;
            previous_arrow = Some(order);
        }
        let mut path_keys = BTreeSet::new();
        let mut previous_path = None;
        for record in &self.paths {
            let order = (scope_order(record.scope), record.path.record_index);
            if record.stage != self.stage
                || record.source_sha256 == Digest::ZERO
                || !valid_stage_or_room_scope(record.scope)
                || !path_keys.insert((record.source_sha256, record.path.record_index))
                || previous_path.is_some_and(|previous| previous >= order)
            {
                return Err(PlannerContractError::new(
                    "orig_world.paths",
                    "must be ordered unique records with valid stage or room scope",
                ));
            }
            validate_path_raw(&record.path)?;
            previous_path = Some(order);
        }
        let mut point_keys = BTreeSet::new();
        let mut previous_point = None;
        for record in &self.path_points {
            let order = (scope_order(record.scope), record.point.record_index);
            if record.stage != self.stage
                || record.source_sha256 == Digest::ZERO
                || !valid_stage_or_room_scope(record.scope)
                || !point_keys.insert((record.source_sha256, record.point.record_index))
                || previous_point.is_some_and(|previous| previous >= order)
            {
                return Err(PlannerContractError::new(
                    "orig_world.path_points",
                    "must be ordered unique records with valid stage or room scope",
                ));
            }
            validate_path_point_raw(&record.point)?;
            previous_point = Some(order);
        }
        Ok(())
    }
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
    pub stage_metadata: Vec<NativeStageMetadata>,
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

        let built = by_stage
            .into_iter()
            .map(|(stage, archives)| -> Result<_, PlannerContractError> {
                Ok((
                    build_inventory(&stage, archives.clone())?,
                    build_stage_metadata(&stage, archives)?,
                ))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let (inventories, stage_metadata) = built.into_iter().unzip();
        let result = Self {
            schema: EXTRACTED_ORIG_WORLD_INVENTORIES_SCHEMA.into(),
            content_sha256: bundle.content.digest()?,
            game_data_sha256: bundle.content.fingerprint.game_data_sha256,
            source_bundle_sha256: bundle.digest()?,
            coverage: expected_coverage(),
            inventories,
            stage_metadata,
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
                "v5 requires complete chunk, placement, SCLS, and map/room metadata coverage and unavailable collision coverage",
            ));
        }
        if self.inventories.is_empty() || self.inventories.len() > 256 {
            return Err(PlannerContractError::new(
                "orig_world.inventories",
                "must contain between 1 and 256 stages",
            ));
        }
        if self.stage_metadata.len() != self.inventories.len() {
            return Err(PlannerContractError::new(
                "orig_world.stage_metadata",
                "must contain exactly one record per inventory stage",
            ));
        }
        let mut previous = None;
        for (inventory, metadata) in self.inventories.iter().zip(&self.stage_metadata) {
            if previous.is_some_and(|stage: &str| stage >= inventory.stage.as_str()) {
                return Err(PlannerContractError::new(
                    "orig_world.inventories",
                    "must be unique and sorted by stage",
                ));
            }
            validate_native_inventory(inventory)?;
            validate_stage_metadata(inventory, metadata)?;
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
        map_room_metadata: NativeWorldCoverageStatus::Complete,
        collision: NativeWorldCoverageStatus::Unavailable,
    }
}

fn build_stage_metadata(
    stage: &str,
    mut archives: Vec<&ExtractedOrigStageArchive>,
) -> Result<NativeStageMetadata, PlannerContractError> {
    archives.sort_by_key(|archive| {
        archive_scope(&archive.relative_path, &archive.resource_name)
            .map(|(_, scope)| scope_order(scope))
            .unwrap_or((2, i16::MAX))
    });
    let mut room_transforms = Vec::new();
    let mut file_lists = Vec::new();
    let mut room_reads = Vec::new();
    let mut cameras = Vec::new();
    let mut camera_arrows = Vec::new();
    let mut paths = Vec::new();
    let mut path_points = Vec::new();
    for archive in archives {
        let (archive_stage, scope) = archive_scope(&archive.relative_path, &archive.resource_name)?;
        if archive_stage != stage {
            return Err(PlannerContractError::new(
                "orig_world.metadata.stage",
                "does not match its archive stage",
            ));
        }
        room_transforms.extend(
            archive
                .stage
                .room_transforms
                .iter()
                .cloned()
                .map(|transform| NativeRoomTransformRecord {
                    stage: stage.into(),
                    source_sha256: archive.resource_sha256,
                    scope,
                    transform,
                }),
        );
        file_lists.extend(archive.stage.file_lists.iter().cloned().map(|file_list| {
            NativeFileListRecord {
                stage: stage.into(),
                source_sha256: archive.resource_sha256,
                scope,
                file_list,
            }
        }));
        room_reads.extend(
            archive
                .stage
                .room_read_table
                .iter()
                .cloned()
                .map(|room_read| NativeRoomReadRecord {
                    stage: stage.into(),
                    source_sha256: archive.resource_sha256,
                    scope,
                    room_read,
                }),
        );
        cameras.extend(
            archive
                .stage
                .cameras
                .iter()
                .cloned()
                .map(|camera| NativeCameraRecord {
                    stage: stage.into(),
                    source_sha256: archive.resource_sha256,
                    scope,
                    camera,
                }),
        );
        camera_arrows.extend(archive.stage.camera_arrows.iter().cloned().map(|arrow| {
            NativeCameraArrowRecord {
                stage: stage.into(),
                source_sha256: archive.resource_sha256,
                scope,
                arrow,
            }
        }));
        paths.extend(
            archive
                .stage
                .paths
                .iter()
                .cloned()
                .map(|path| NativePathRecord {
                    stage: stage.into(),
                    source_sha256: archive.resource_sha256,
                    scope,
                    path,
                }),
        );
        path_points.extend(archive.stage.path_points.iter().cloned().map(|point| {
            NativePathPointRecord {
                stage: stage.into(),
                source_sha256: archive.resource_sha256,
                scope,
                point,
            }
        }));
    }
    Ok(NativeStageMetadata {
        stage: stage.into(),
        room_transforms,
        file_lists,
        room_reads,
        cameras,
        camera_arrows,
        paths,
        path_points,
    })
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

fn valid_stage_or_room_scope(scope: SourceScope) -> bool {
    matches!(
        scope,
        SourceScope {
            kind: SourceKind::Stage,
            room: None
        } | SourceScope {
            kind: SourceKind::Room,
            room: Some(0..=i8::MAX)
        }
    )
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

#[cfg(test)]
#[path = "orig_world_tests.rs"]
mod tests;
