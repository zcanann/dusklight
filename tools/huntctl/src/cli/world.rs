//! Static world inventory, geometry inspection, and spatial-query adapters.

use crate::{flag, option, repeated_option, required_path, usage_error, usize_option};
use huntctl::Digest;
use huntctl::actor_profile_catalog::ActorProfileCatalog;
use huntctl::content_store::{ContentKind, ContentStore};
use huntctl::stage_boot_catalog::{StageBootCatalog, StageInventoryStatus};
use huntctl::world_context::WorldContext;
use huntctl::world_geometry::{KclPlc, Vec3, extract_rarc_resource, query_prism_point};
use huntctl::world_inventory::WorldInventory;
use huntctl::world_spatial::{
    Aabb3, WorldAabbQueryRequest, WorldPointQueryRequest, WorldRayQueryRequest, WorldSpatialIndex,
    WorldSurfaceFilter,
};
use serde_json::json;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) fn command_world(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("boot-catalog") => {
            let stage_root = required_path(&args[1..], "--stage-root")?;
            let known_loader = option(&args[1..], "--known-loader").map(PathBuf::from);
            let output = required_path(&args[1..], "--output")?;
            let catalog = StageBootCatalog::build(&stage_root, known_loader.as_deref())?;
            let bytes = catalog.canonical_bytes()?;
            let digest = catalog.digest()?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            fs::write(&output, &bytes)?;
            let artifact_store = option(&args[1..], "--artifact-store")
                .map(PathBuf::from)
                .unwrap_or_else(|| output.parent().unwrap_or(Path::new(".")).join("content"));
            let content_blob = ContentStore::initialize(&artifact_store)?
                .put_bytes(&bytes, ContentKind::StageBootCatalog)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": catalog.schema,
                    "output": output,
                    "artifact_store": artifact_store,
                    "content_blob": content_blob,
                    "sha256": digest,
                    "bytes": bytes.len(),
                    "stages": catalog.stages.len(),
                    "complete_stages": catalog.stages.iter()
                        .filter(|stage| stage.inventory_status == StageInventoryStatus::Complete)
                        .count(),
                    "unreadable_stages": catalog.stages.iter()
                        .filter(|stage| stage.inventory_status == StageInventoryStatus::Unreadable)
                        .count(),
                    "loader_only_stages": catalog.stages.iter()
                        .filter(|stage| stage.inventory_status == StageInventoryStatus::LoaderOnly)
                        .count(),
                    "candidates": catalog.candidates.len(),
                }))?
            );
            Ok(())
        }
        Some("inventory") => {
            let stage_dir = required_path(&args[1..], "--stage-dir")?;
            let stage = option(&args[1..], "--stage").ok_or("missing required --stage ID")?;
            let output = required_path(&args[1..], "--output")?;
            let inventory = WorldInventory::build(&stage_dir, &stage)?;
            let bytes = inventory.canonical_bytes()?;
            let digest = inventory.digest()?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            fs::write(&output, &bytes)?;
            let artifact_store = option(&args[1..], "--artifact-store")
                .map(PathBuf::from)
                .unwrap_or_else(|| output.parent().unwrap_or(Path::new(".")).join("content"));
            let content_blob = ContentStore::initialize(&artifact_store)?
                .put_bytes(&bytes, ContentKind::WorldInventory)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": inventory.schema,
                    "stage": inventory.stage,
                    "output": output,
                    "artifact_store": artifact_store,
                    "content_blob": content_blob,
                    "sha256": digest,
                    "bytes": bytes.len(),
                    "sources": inventory.sources.len(),
                    "chunks": inventory.chunks.len(),
                    "placements": inventory.placements.len(),
                    "player_spawns": inventory.player_spawns.len(),
                    "exits": inventory.exits.len(),
                    "collisions": inventory.collisions.len(),
                    "load_triggers": inventory.load_triggers.len(),
                }))?
            );
            Ok(())
        }
        Some("context") => {
            let output = required_path(&args[1..], "--output")?;
            let game_data_sha256: Digest = option(&args[1..], "--game-data-sha256")
                .ok_or("missing required --game-data-sha256 SHA256")?
                .parse()?;
            let inventory_paths = repeated_option(&args[1..], "--inventory");
            if inventory_paths.is_empty() {
                return Err("world context requires at least one --inventory FILE".into());
            }
            let inventories = inventory_paths
                .iter()
                .map(|path| WorldInventory::read_canonical(Path::new(path)))
                .collect::<Result<Vec<_>, _>>()?;
            let context = WorldContext::build(game_data_sha256, &inventories)?;
            let bytes = context.canonical_bytes()?;
            let digest = context.digest()?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            fs::write(&output, &bytes)?;
            let artifact_store = option(&args[1..], "--artifact-store")
                .map(PathBuf::from)
                .unwrap_or_else(|| output.parent().unwrap_or(Path::new(".")).join("content"));
            let content_blob = ContentStore::initialize(&artifact_store)?
                .put_bytes(&bytes, ContentKind::WorldContext)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": context.schema,
                    "game_data_sha256": context.game_data_sha256,
                    "stages": context.stages,
                    "output": output,
                    "artifact_store": artifact_store,
                    "content_blob": content_blob,
                    "sha256": digest,
                    "bytes": bytes.len(),
                }))?
            );
            Ok(())
        }
        Some("profile-catalog") => {
            let input = required_path(&args[1..], "--input")?;
            let bytes = fs::read(&input)?;
            let catalog = ActorProfileCatalog::decode_canonical(&bytes)?;
            let artifact_store = option(&args[1..], "--artifact-store")
                .map(PathBuf::from)
                .unwrap_or_else(|| input.parent().unwrap_or(Path::new(".")).join("content"));
            let content_blob = ContentStore::initialize(&artifact_store)?
                .put_bytes(&bytes, ContentKind::ActorProfileCatalog)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": catalog.schema,
                    "identity": catalog.identity,
                    "sha256": catalog.digest()?,
                    "profiles": catalog.profiles.len(),
                    "present_profiles": catalog.profiles.iter()
                        .filter(|profile| profile.present).count(),
                    "actor_profiles": catalog.profiles.iter()
                        .filter(|profile| profile.is_actor == Some(true)).count(),
                    "input": input,
                    "artifact_store": artifact_store,
                    "content_blob": content_blob,
                }))?
            );
            Ok(())
        }
        Some("spatial-index") => {
            let output = required_path(&args[1..], "--output")?;
            let inventory = load_world_inventory(&args[1..])?;
            let index = WorldSpatialIndex::build(&inventory)?;
            let bytes = index.artifact().canonical_bytes()?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            fs::write(&output, &bytes)?;
            let artifact_store = option(&args[1..], "--artifact-store")
                .map(PathBuf::from)
                .unwrap_or_else(|| output.parent().unwrap_or(Path::new(".")).join("content"));
            let content_blob = ContentStore::initialize(&artifact_store)?
                .put_bytes(&bytes, ContentKind::WorldSpatialIndex)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": index.artifact().schema,
                    "stage": inventory.stage,
                    "inventory_sha256": inventory.digest()?,
                    "spatial_index_sha256": index.artifact_digest()?,
                    "bytes": bytes.len(),
                    "rooms": index.artifact().rooms.len(),
                    "indexed_surfaces": index.artifact().rooms.iter()
                        .map(|room| room.primitive_ids.len()).sum::<usize>(),
                    "excluded_surfaces": index.artifact().excluded.len(),
                    "output": output,
                    "artifact_store": artifact_store,
                    "content_blob": content_blob,
                }))?
            );
            Ok(())
        }
        Some("query") => command_world_query(&args[1..]),
        Some("kcl") => {
            let prism_index: u16 = option(&args[1..], "--prism")
                .ok_or("missing required --prism INDEX")?
                .parse()?;
            let archive_path = option(&args[1..], "--archive").map(PathBuf::from);
            let kcl_path = option(&args[1..], "--kcl").map(PathBuf::from);
            let plc_path = option(&args[1..], "--plc").map(PathBuf::from);

            let (kcl, plc, source) = match (archive_path, kcl_path, plc_path) {
                (Some(archive), None, None) => {
                    let kcl_name =
                        option(&args[1..], "--kcl-name").unwrap_or_else(|| "room.kcl".into());
                    let plc_name =
                        option(&args[1..], "--plc-name").unwrap_or_else(|| "room.plc".into());
                    let archive_bytes = fs::read(&archive)?;
                    let kcl = extract_rarc_resource(&archive_bytes, &kcl_name)?;
                    let plc = extract_rarc_resource(&archive_bytes, &plc_name)?;
                    let source = json!({
                        "kind": "rarc",
                        "archive": archive,
                        "kcl_resource": kcl_name,
                        "plc_resource": plc_name,
                    });
                    (kcl, plc, source)
                }
                (None, Some(kcl), Some(plc)) => {
                    let source = json!({
                        "kind": "loose_files",
                        "kcl": kcl,
                        "plc": plc,
                    });
                    (fs::read(&kcl)?, fs::read(&plc)?, source)
                }
                _ => {
                    return Err(
                        "world kcl requires either --archive PATH or both --kcl PATH --plc PATH"
                            .into(),
                    );
                }
            };
            let inspection = KclPlc::parse(&kcl, &plc)?.inspect_prism(prism_index)?;
            let point_query = option(&args[1..], "--point")
                .map(|value| parse_world_point(&value))
                .transpose()?
                .map(|point| query_prism_point(&inspection.prism, point))
                .transpose()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "source": source,
                    "inspection": inspection,
                    "point_query": point_query,
                }))?
            );
            Ok(())
        }
        _ => usage_error(),
    }
}

fn command_world_query(args: &[String]) -> Result<(), Box<dyn Error>> {
    let operation = args
        .first()
        .map(String::as_str)
        .ok_or("world query requires point, aabb, or ray as its operation")?;
    let query_args = &args[1..];
    let room: i8 = option(query_args, "--room")
        .ok_or("missing required --room N (coordinates are room-scoped)")?
        .parse()?;
    let limit = usize_option(query_args, "--limit", 8)?;
    let filter = WorldSurfaceFilter {
        room,
        load_triggers_only: flag(query_args, "--load-triggers-only"),
        trigger_stable_id: option(query_args, "--trigger-id"),
        destination_stage: option(query_args, "--destination-stage"),
        destination_room: option(query_args, "--destination-room")
            .map(|value| value.parse())
            .transpose()?,
        destination_point: option(query_args, "--destination-point")
            .map(|value| value.parse())
            .transpose()?,
    };
    let inventory = load_world_inventory(query_args)?;
    let index = WorldSpatialIndex::build(&inventory)?;
    match operation {
        "point" => {
            let point = parse_world_vec3(
                &option(query_args, "--point").ok_or("missing required --point X,Y,Z")?,
                "--point",
            )?;
            let max_distance = option(query_args, "--max-distance")
                .map(|value| value.parse())
                .transpose()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&index.point_query(WorldPointQueryRequest {
                    point,
                    max_distance,
                    limit,
                    filter,
                })?)?
            );
        }
        "aabb" => {
            let min = parse_world_vec3(
                &option(query_args, "--min").ok_or("missing required --min X,Y,Z")?,
                "--min",
            )?;
            let max = parse_world_vec3(
                &option(query_args, "--max").ok_or("missing required --max X,Y,Z")?,
                "--max",
            )?;
            println!(
                "{}",
                serde_json::to_string_pretty(&index.aabb_query(WorldAabbQueryRequest {
                    bounds: Aabb3::new(min, max)?,
                    limit,
                    filter,
                })?)?
            );
        }
        "ray" => {
            let origin = parse_world_vec3(
                &option(query_args, "--origin").ok_or("missing required --origin X,Y,Z")?,
                "--origin",
            )?;
            let direction = parse_world_vec3(
                &option(query_args, "--direction").ok_or("missing required --direction X,Y,Z")?,
                "--direction",
            )?;
            let max_distance: f32 = option(query_args, "--max-distance")
                .ok_or("missing required --max-distance DISTANCE")?
                .parse()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&index.ray_query(WorldRayQueryRequest {
                    origin,
                    direction,
                    max_distance,
                    limit,
                    filter,
                })?)?
            );
        }
        _ => return Err("world query operation must be point, aabb, or ray".into()),
    }
    Ok(())
}

fn load_world_inventory(args: &[String]) -> Result<WorldInventory, Box<dyn Error>> {
    let inventory = option(args, "--inventory").map(PathBuf::from);
    let stage_dir = option(args, "--stage-dir").map(PathBuf::from);
    let stage = option(args, "--stage");
    match (inventory, stage_dir, stage) {
        (Some(path), None, None) => Ok(WorldInventory::read_canonical(&path)?),
        (None, Some(stage_dir), Some(stage)) => Ok(WorldInventory::build(&stage_dir, &stage)?),
        _ => Err(
            "world operation requires either --inventory INVENTORY.json or both --stage-dir STAGE_DIR --stage STAGE_ID"
                .into(),
        ),
    }
}

fn parse_world_point(value: &str) -> Result<Vec3, Box<dyn Error>> {
    parse_world_vec3(value, "--point")
}

fn parse_world_vec3(value: &str, option_name: &str) -> Result<Vec3, Box<dyn Error>> {
    let components = value.split(',').collect::<Vec<_>>();
    if components.len() != 3 {
        return Err(format!("{option_name} must be exactly X,Y,Z").into());
    }
    let point = Vec3 {
        x: components[0].parse()?,
        y: components[1].parse()?,
        z: components[2].parse()?,
    };
    if !point.x.is_finite() || !point.y.is_finite() || !point.z.is_finite() {
        return Err(format!("{option_name} components must be finite").into());
    }
    Ok(point)
}
