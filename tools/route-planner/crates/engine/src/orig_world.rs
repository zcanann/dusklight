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

fn validate_stage_metadata(
    inventory: &WorldInventory,
    metadata: &NativeStageMetadata,
) -> Result<(), PlannerContractError> {
    metadata.validate_records()?;
    if metadata.stage != inventory.stage {
        return Err(PlannerContractError::new(
            "orig_world.stage_metadata.stage",
            "does not match its inventory stage",
        ));
    }
    let source_scopes = inventory
        .sources
        .iter()
        .map(|source| (source.stage_data_sha256, source.scope))
        .collect::<BTreeMap<_, _>>();
    let chunk_count = |digest: Digest, tag: &str| {
        inventory
            .chunks
            .iter()
            .find(|chunk| chunk.source_sha256 == digest && chunk.tag == tag)
            .map(|chunk| chunk.record_count)
            .unwrap_or(0)
    };

    for record in &metadata.room_transforms {
        let transform = &record.transform;
        if record.stage != inventory.stage
            || source_scopes.get(&record.source_sha256) != Some(&record.scope)
            || transform.record_index as usize >= chunk_count(record.source_sha256, "MULT")
        {
            return Err(PlannerContractError::new(
                "orig_world.room_transforms",
                "contains a MULT record outside its exact source chunk",
            ));
        }
    }

    for record in &metadata.file_lists {
        let file_list = &record.file_list;
        if record.stage != inventory.stage
            || source_scopes.get(&record.source_sha256) != Some(&record.scope)
            || file_list.record_index as usize >= chunk_count(record.source_sha256, "FILI")
        {
            return Err(PlannerContractError::new(
                "orig_world.file_lists",
                "contains a FILI record outside its exact source chunk",
            ));
        }
    }

    for record in &metadata.room_reads {
        if record.stage != inventory.stage
            || source_scopes.get(&record.source_sha256) != Some(&record.scope)
            || record.room_read.room_index as usize >= chunk_count(record.source_sha256, "RTBL")
        {
            return Err(PlannerContractError::new(
                "orig_world.room_reads",
                "contains an RTBL record outside its exact source chunk",
            ));
        }
    }

    for record in &metadata.cameras {
        let arrow_count = chunk_count(record.source_sha256, "RARO");
        if record.stage != inventory.stage
            || source_scopes.get(&record.source_sha256) != Some(&record.scope)
            || record.camera.record_index as usize >= chunk_count(record.source_sha256, "RCAM")
            || usize::from(record.camera.arrow_index) >= arrow_count
        {
            return Err(PlannerContractError::new(
                "orig_world.cameras",
                "contains an RCAM record outside its exact source chunk or referencing a missing RARO record",
            ));
        }
    }

    for record in &metadata.camera_arrows {
        if record.stage != inventory.stage
            || source_scopes.get(&record.source_sha256) != Some(&record.scope)
            || record.arrow.record_index as usize >= chunk_count(record.source_sha256, "RARO")
        {
            return Err(PlannerContractError::new(
                "orig_world.camera_arrows",
                "contains a RARO record outside its exact source chunk",
            ));
        }
    }

    for record in &metadata.paths {
        let path_count = chunk_count(record.source_sha256, "RPAT");
        let point_count = chunk_count(record.source_sha256, "RPPN");
        let first = record.path.first_point_index as usize;
        let end = first
            .checked_add(usize::from(record.path.point_count))
            .ok_or_else(|| PlannerContractError::new("orig_world.paths", "point range overflow"))?;
        if record.stage != inventory.stage
            || source_scopes.get(&record.source_sha256) != Some(&record.scope)
            || record.path.record_index as usize >= path_count
            || end > point_count
            || record
                .path
                .next_path_index
                .is_some_and(|next| usize::from(next) >= path_count)
        {
            return Err(PlannerContractError::new(
                "orig_world.paths",
                "contains an RPAT record outside its source chunk or referencing an absent path/point",
            ));
        }
    }

    for record in &metadata.path_points {
        if record.stage != inventory.stage
            || source_scopes.get(&record.source_sha256) != Some(&record.scope)
            || record.point.record_index as usize >= chunk_count(record.source_sha256, "RPPN")
        {
            return Err(PlannerContractError::new(
                "orig_world.path_points",
                "contains an RPPN record outside its exact source chunk",
            ));
        }
    }

    let expected_transforms = inventory
        .chunks
        .iter()
        .filter(|chunk| chunk.tag == "MULT")
        .map(|chunk| chunk.record_count)
        .sum::<usize>();
    let expected_file_lists = inventory
        .chunks
        .iter()
        .filter(|chunk| chunk.tag == "FILI")
        .map(|chunk| chunk.record_count)
        .sum::<usize>();
    let expected_room_reads = inventory
        .chunks
        .iter()
        .filter(|chunk| chunk.tag == "RTBL")
        .map(|chunk| chunk.record_count)
        .sum::<usize>();
    let expected_cameras = inventory
        .chunks
        .iter()
        .filter(|chunk| chunk.tag == "RCAM")
        .map(|chunk| chunk.record_count)
        .sum::<usize>();
    let expected_camera_arrows = inventory
        .chunks
        .iter()
        .filter(|chunk| chunk.tag == "RARO")
        .map(|chunk| chunk.record_count)
        .sum::<usize>();
    let expected_paths = inventory
        .chunks
        .iter()
        .filter(|chunk| chunk.tag == "RPAT")
        .map(|chunk| chunk.record_count)
        .sum::<usize>();
    let expected_path_points = inventory
        .chunks
        .iter()
        .filter(|chunk| chunk.tag == "RPPN")
        .map(|chunk| chunk.record_count)
        .sum::<usize>();
    if metadata.room_transforms.len() != expected_transforms
        || metadata.file_lists.len() != expected_file_lists
        || metadata.room_reads.len() != expected_room_reads
        || metadata.cameras.len() != expected_cameras
        || metadata.camera_arrows.len() != expected_camera_arrows
        || metadata.paths.len() != expected_paths
        || metadata.path_points.len() != expected_path_points
    {
        return Err(PlannerContractError::new(
            "orig_world.stage_metadata",
            "does not completely cover its recognized MULT, FILI, RTBL, RCAM, RARO, RPAT, and RPPN chunks",
        ));
    }
    Ok(())
}

fn validate_room_transform_raw(
    transform: &ExtractedRoomTransform,
) -> Result<(), PlannerContractError> {
    let raw = decode_hex_exact(&transform.raw_hex, 12, "orig_world.mult.raw_hex")?;
    let translation_x = f32::from_bits(u32::from_be_bytes(raw[0..4].try_into().unwrap()));
    let translation_z = f32::from_bits(u32::from_be_bytes(raw[4..8].try_into().unwrap()));
    if !translation_x.is_finite()
        || !translation_z.is_finite()
        || translation_x.to_bits() != transform.translation_xz[0].to_bits()
        || translation_z.to_bits() != transform.translation_xz[1].to_bits()
        || i16::from_be_bytes(raw[8..10].try_into().unwrap()) != transform.angle_y
        || raw[10] != transform.room
        || raw[11] != transform.trailing_byte
    {
        return Err(PlannerContractError::new(
            "orig_world.room_transforms",
            "decoded fields do not match the retained raw MULT record",
        ));
    }
    Ok(())
}

fn validate_file_list_raw(file_list: &ExtractedFileList) -> Result<(), PlannerContractError> {
    let raw = decode_hex_exact(&file_list.raw_hex, 32, "orig_world.fili.raw_hex")?;
    let parameters = u32::from_be_bytes(raw[0..4].try_into().unwrap());
    let sea_level = f32::from_bits(u32::from_be_bytes(raw[4..8].try_into().unwrap()));
    let unknown_08 = f32::from_bits(u32::from_be_bytes(raw[8..12].try_into().unwrap()));
    let unknown_0c = f32::from_bits(u32::from_be_bytes(raw[12..16].try_into().unwrap()));
    let message_id = u16::from_be_bytes(raw[0x1c..0x1e].try_into().unwrap());
    if !sea_level.is_finite()
        || !unknown_08.is_finite()
        || !unknown_0c.is_finite()
        || parameters != file_list.parameters
        || sea_level.to_bits() != file_list.sea_level.to_bits()
        || unknown_08.to_bits() != file_list.unknown_float_08.to_bits()
        || unknown_0c.to_bits() != file_list.unknown_float_0c.to_bits()
        || hex_bytes(&raw[0x10..0x1a]) != file_list.unknown_bytes_10_19_hex
        || ((parameters >> 3) & 7) as u8 != file_list.minimap_style
        || (parameters & 0x2000_0000 != 0) != file_list.enemy_appear_flag
        || ((parameters >> 18) & 3) as u8 != file_list.global_wind_level
        || ((parameters >> 15) & 7) as u8 != file_list.global_wind_direction
        || ((parameters >> 7) & 0xff) as u8 != file_list.grass_light
        || raw[0x1a] != file_list.default_camera
        || raw[0x1b] != file_list.bit_switch
        || message_id != file_list.message_id
    {
        return Err(PlannerContractError::new(
            "orig_world.file_lists",
            "decoded fields do not match the retained raw FILI record",
        ));
    }
    Ok(())
}

fn validate_room_read_raw(room_read: &ExtractedRoomRead) -> Result<(), PlannerContractError> {
    let header = decode_hex_exact(
        &room_read.raw_header_hex,
        8,
        "orig_world.rtbl.raw_header_hex",
    )?;
    let room_list = decode_hex_exact(
        &room_read.raw_room_list_hex,
        room_read.load_rooms.len(),
        "orig_world.rtbl.raw_room_list_hex",
    )?;
    if room_read.record_offset == 0
        || (room_read.room_list_offset == 0 && !room_list.is_empty())
        || usize::from(header[0]) != room_read.load_rooms.len()
        || header[1] != room_read.reverb_raw
        || header[1] & 0x7f != room_read.reverb
        || header[2] != room_read.flags_raw
        || header[2] & 3 != room_read.time_pass
        || (header[2] & 8 != 0) != room_read.vrbox_enabled
        || header[3] != room_read.padding
        || u32::from_be_bytes(header[4..8].try_into().unwrap()) != room_read.room_list_offset
    {
        return Err(PlannerContractError::new(
            "orig_world.room_reads",
            "decoded fields do not match the retained raw RTBL header",
        ));
    }
    for (decoded, raw) in room_read.load_rooms.iter().zip(room_list) {
        if decoded.raw != raw
            || decoded.room != raw & 0x3f
            || decoded.load_background != (raw & 0x80 != 0)
            || decoded.unknown_bit_6 != (raw & 0x40 != 0)
        {
            return Err(PlannerContractError::new(
                "orig_world.room_reads.load_rooms",
                "decoded fields do not match the retained raw room-load byte",
            ));
        }
    }
    Ok(())
}

fn validate_camera_raw(camera: &ExtractedCamera) -> Result<(), PlannerContractError> {
    let raw = decode_hex_exact(&camera.raw_hex, 0x18, "orig_world.rcam.raw_hex")?;
    let type_end = raw[..0x10]
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(0x10);
    let raw_type_index = u16::from_be_bytes(raw[0x16..0x18].try_into().unwrap());
    if type_end == 0
        || !raw[..type_end].iter().all(u8::is_ascii_graphic)
        || &raw[..type_end] != camera.camera_type.as_bytes()
        || raw[0x10] != camera.arrow_index
        || raw[0x11] != camera.field_of_view_y
        || raw[0x12] != camera.argument_0
        || raw[0x13] != camera.argument_1
        || u16::from_be_bytes(raw[0x14..0x16].try_into().unwrap()) != camera.argument_2
        || (raw_type_index != u16::MAX).then_some(raw_type_index) != camera.camera_type_index
    {
        return Err(PlannerContractError::new(
            "orig_world.cameras",
            "decoded fields do not match the retained raw RCAM record",
        ));
    }
    Ok(())
}

fn validate_camera_arrow_raw(arrow: &ExtractedCameraArrow) -> Result<(), PlannerContractError> {
    let raw = decode_hex_exact(&arrow.raw_hex, 0x14, "orig_world.raro.raw_hex")?;
    let position = [
        f32::from_bits(u32::from_be_bytes(raw[0..4].try_into().unwrap())),
        f32::from_bits(u32::from_be_bytes(raw[4..8].try_into().unwrap())),
        f32::from_bits(u32::from_be_bytes(raw[8..12].try_into().unwrap())),
    ];
    let angle = [
        i16::from_be_bytes(raw[0x0c..0x0e].try_into().unwrap()),
        i16::from_be_bytes(raw[0x0e..0x10].try_into().unwrap()),
        i16::from_be_bytes(raw[0x10..0x12].try_into().unwrap()),
    ];
    if !position.iter().all(|coordinate| coordinate.is_finite())
        || position
            .iter()
            .zip(arrow.position)
            .any(|(raw, decoded)| raw.to_bits() != decoded.to_bits())
        || angle != arrow.angle
        || i16::from_be_bytes(raw[0x12..0x14].try_into().unwrap()) != arrow.trailing_i16
    {
        return Err(PlannerContractError::new(
            "orig_world.camera_arrows",
            "decoded fields do not match the retained raw RARO record",
        ));
    }
    Ok(())
}

fn validate_path_raw(path: &ExtractedPath) -> Result<(), PlannerContractError> {
    let raw = decode_hex_exact(&path.raw_hex, 0x0c, "orig_world.rpat.raw_hex")?;
    let next_raw = u16::from_be_bytes(raw[2..4].try_into().unwrap());
    let point_offset = u32::from_be_bytes(raw[8..12].try_into().unwrap());
    if u16::from_be_bytes(raw[0..2].try_into().unwrap()) != path.point_count
        || (next_raw != u16::MAX).then_some(next_raw) != path.next_path_index
        || raw[4] != path.path_argument
        || (raw[5] & 1 != 0) != path.closed
        || raw[5] != path.closed_raw
        || (raw[6] != u8::MAX).then_some(raw[6]) != path.switch_no
        || raw[7] != path.unknown_07
        || point_offset != path.point_offset
        || point_offset % 0x10 != 0
        || point_offset / 0x10 != path.first_point_index
    {
        return Err(PlannerContractError::new(
            "orig_world.paths",
            "decoded fields do not match the retained raw RPAT record",
        ));
    }
    Ok(())
}

fn validate_path_point_raw(point: &ExtractedPathPoint) -> Result<(), PlannerContractError> {
    let raw = decode_hex_exact(&point.raw_hex, 0x10, "orig_world.rppn.raw_hex")?;
    let position = [
        f32::from_bits(u32::from_be_bytes(raw[4..8].try_into().unwrap())),
        f32::from_bits(u32::from_be_bytes(raw[8..12].try_into().unwrap())),
        f32::from_bits(u32::from_be_bytes(raw[12..16].try_into().unwrap())),
    ];
    if [raw[3], raw[0], raw[1], raw[2]] != point.arguments
        || !position.iter().all(|coordinate| coordinate.is_finite())
        || position
            .iter()
            .zip(point.position)
            .any(|(raw, decoded)| raw.to_bits() != decoded.to_bits())
    {
        return Err(PlannerContractError::new(
            "orig_world.path_points",
            "decoded fields do not match the retained raw RPPN record",
        ));
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
        "MULT" => Some(12),
        "FILI" => Some(32),
        "RCAM" => Some(24),
        "RARO" => Some(20),
        "RPAT" => Some(12),
        "RPPN" => Some(16),
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

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
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
        ExtractedCamera, ExtractedCameraArrow, ExtractedLoadedRoom, ExtractedPath,
        ExtractedPathPoint, ExtractedRoomRead, ExtractedStageChunk, ExtractedStageData,
        extract_unique_rarc_resource, parse_stage_data,
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
                chunks: vec![
                    ExtractedStageChunk {
                        tag: "ACTR".into(),
                        record_count: 1,
                        data_offset: 64,
                        recognized_record_size: Some(32),
                    },
                    ExtractedStageChunk {
                        tag: "RTBL".into(),
                        record_count: 1,
                        data_offset: 96,
                        recognized_record_size: None,
                    },
                ],
                stage_information: None,
                room_transforms: Vec::new(),
                file_lists: Vec::new(),
                room_read_table: vec![ExtractedRoomRead {
                    room_index: 0,
                    record_offset: 100,
                    room_list_offset: 128,
                    reverb: 5,
                    reverb_raw: 5,
                    time_pass: 3,
                    vrbox_enabled: true,
                    flags_raw: 0x0b,
                    padding: 0,
                    load_rooms: vec![ExtractedLoadedRoom {
                        room: 2,
                        load_background: true,
                        unknown_bit_6: false,
                        raw: 0x82,
                    }],
                    raw_header_hex: "01050b0000000080".into(),
                    raw_room_list_hex: "82".into(),
                }],
                cameras: Vec::new(),
                camera_arrows: Vec::new(),
                paths: Vec::new(),
                path_points: Vec::new(),
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
                    ExtractedStageChunk {
                        tag: "RCAM".into(),
                        record_count: 1,
                        data_offset: 144,
                        recognized_record_size: Some(24),
                    },
                    ExtractedStageChunk {
                        tag: "RARO".into(),
                        record_count: 1,
                        data_offset: 168,
                        recognized_record_size: Some(20),
                    },
                    ExtractedStageChunk {
                        tag: "RPAT".into(),
                        record_count: 1,
                        data_offset: 188,
                        recognized_record_size: Some(12),
                    },
                    ExtractedStageChunk {
                        tag: "RPPN".into(),
                        record_count: 1,
                        data_offset: 200,
                        recognized_record_size: Some(16),
                    },
                ],
                stage_information: None,
                room_transforms: Vec::new(),
                file_lists: Vec::new(),
                room_read_table: Vec::new(),
                cameras: vec![ExtractedCamera {
                    record_index: 0,
                    camera_type: "FixedFrame".into(),
                    arrow_index: 0,
                    field_of_view_y: 55,
                    argument_0: 2,
                    argument_1: 3,
                    argument_2: 0xa123,
                    camera_type_index: None,
                    raw_hex: "46697865644672616d6500000000000000370203a123ffff".into(),
                }],
                camera_arrows: vec![ExtractedCameraArrow {
                    record_index: 0,
                    position: [10.5, -20.0, 30.25],
                    angle: [-1024, 0x4000, 7],
                    trailing_i16: -1,
                    raw_hex: "41280000c1a0000041f20000fc0040000007ffff".into(),
                }],
                paths: vec![ExtractedPath {
                    record_index: 0,
                    point_count: 1,
                    next_path_index: None,
                    path_argument: 4,
                    closed: false,
                    closed_raw: 0,
                    switch_no: None,
                    unknown_07: 0xbb,
                    point_offset: 0,
                    first_point_index: 0,
                    raw_hex: "0001ffff0400ffbb00000000".into(),
                }],
                path_points: vec![ExtractedPathPoint {
                    record_index: 0,
                    arguments: [10, 11, 12, 13],
                    position: [7.0, 8.0, 9.0],
                    raw_hex: "0b0c0d0a40e000004100000041100000".into(),
                }],
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
            stage_metadata: vec![build_stage_metadata("F_SP00", vec![&room, &stage]).unwrap()],
        };
        let bytes = set.canonical_bytes().unwrap();
        assert_eq!(
            ExtractedOrigWorldInventories::decode_canonical(&bytes).unwrap(),
            set
        );
        let mut tampered_path = set.clone();
        tampered_path.stage_metadata[0].paths[0].path.path_argument = 5;
        assert_eq!(
            tampered_path.validate().unwrap_err().field(),
            "orig_world.paths"
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
        assert_eq!(facts.native_stage_metadata, import_set.stage_metadata);
        assert_eq!(facts.native_stage_metadata[0].room_reads.len(), 1);
        assert_eq!(facts.native_stage_metadata[0].cameras.len(), 1);
        assert_eq!(facts.native_stage_metadata[0].camera_arrows.len(), 1);
        assert_eq!(facts.native_stage_metadata[0].paths.len(), 1);
        assert_eq!(facts.native_stage_metadata[0].path_points.len(), 1);
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
    fn rejects_room_metadata_that_disagrees_with_its_raw_record() {
        let mut raw = [0_u8; 12];
        raw[0..4].copy_from_slice(&10.5_f32.to_bits().to_be_bytes());
        raw[4..8].copy_from_slice(&(-2.0_f32).to_bits().to_be_bytes());
        raw[8..10].copy_from_slice(&0x2000_i16.to_be_bytes());
        raw[10] = 3;
        raw[11] = 0xff;
        let mut metadata = NativeStageMetadata {
            stage: "F_SP00".into(),
            room_transforms: vec![NativeRoomTransformRecord {
                stage: "F_SP00".into(),
                source_sha256: digest(1),
                scope: SourceScope {
                    kind: SourceKind::Stage,
                    room: None,
                },
                transform: ExtractedRoomTransform {
                    record_index: 0,
                    room: 3,
                    translation_xz: [10.5, -2.0],
                    angle_y: 0x2000,
                    trailing_byte: 0xff,
                    raw_hex: hex_bytes(&raw),
                },
            }],
            file_lists: Vec::new(),
            room_reads: Vec::new(),
            cameras: Vec::new(),
            camera_arrows: Vec::new(),
            paths: Vec::new(),
            path_points: Vec::new(),
        };
        metadata.validate_records().unwrap();
        metadata.room_transforms[0].transform.room = 4;
        assert_eq!(
            metadata.validate_records().unwrap_err().field(),
            "orig_world.room_transforms"
        );
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

    #[test]
    fn exact_native_map_room_metadata_validates_for_every_stage_when_available() {
        let Some(root) = repository_root() else {
            return;
        };
        let stage_root = root.join("orig/GZ2E01/files/res/Stage");
        if !stage_root.is_dir() {
            return;
        }
        let mut stage_dirs = fs::read_dir(&stage_root)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.is_dir())
            .collect::<Vec<_>>();
        stage_dirs.sort();
        let mut room_reads = 0_usize;
        let mut cameras = 0_usize;
        let mut camera_arrows = 0_usize;
        let mut paths = 0_usize;
        let mut path_points = 0_usize;
        for stage_dir in &stage_dirs {
            let stage = stage_dir.file_name().unwrap().to_str().unwrap();
            let mut archive_paths = fs::read_dir(stage_dir)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .filter(|path| {
                    path.extension().is_some_and(|extension| extension == "arc")
                        && path.file_name().is_some_and(|name| {
                            let name = name.to_string_lossy();
                            name == "STG_00.arc"
                                || name.starts_with('R') && name.ends_with("_00.arc")
                        })
                })
                .collect::<Vec<_>>();
            archive_paths.sort_by_key(|path| {
                let name = path.file_name().unwrap().to_string_lossy();
                if name == "STG_00.arc" {
                    (0, name.into_owned())
                } else {
                    (1, name.into_owned())
                }
            });
            let mut archives = Vec::new();
            for path in archive_paths {
                let name = path.file_name().unwrap().to_str().unwrap();
                let resource = if name == "STG_00.arc" {
                    "stage.dzs"
                } else {
                    "room.dzr"
                };
                let bytes = fs::read(&path).unwrap();
                let resource_bytes = extract_unique_rarc_resource(&bytes, resource).unwrap();
                archives.push(archive(
                    &format!("files/res/Stage/{stage}/{name}"),
                    resource,
                    Digest(Sha256::digest(&resource_bytes).into()),
                    parse_stage_data(&resource_bytes).unwrap(),
                ));
                archives.last_mut().unwrap().archive_sha256 = Digest(Sha256::digest(&bytes).into());
            }
            let refs = archives.iter().collect::<Vec<_>>();
            let inventory = build_inventory(stage, refs.clone()).unwrap();
            let metadata = build_stage_metadata(stage, refs).unwrap();
            validate_stage_metadata(&inventory, &metadata).unwrap();
            room_reads += metadata.room_reads.len();
            cameras += metadata.cameras.len();
            camera_arrows += metadata.camera_arrows.len();
            paths += metadata.paths.len();
            path_points += metadata.path_points.len();
        }
        assert_eq!(stage_dirs.len(), 79);
        assert_eq!(room_reads, 1_652);
        assert_eq!(cameras, 1_260);
        assert_eq!(camera_arrows, 1_260);
        assert_eq!(paths, 2_703);
        assert_eq!(path_points, 16_997);
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
