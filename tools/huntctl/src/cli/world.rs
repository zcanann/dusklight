//! Static world inventory, geometry inspection, and spatial-query adapters.

use crate::{flag, option, required_path, usage_error, usize_option};
use huntctl::content_store::{ContentKind, ContentStore};
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
        Some("spatial-index") => {
            let stage_dir = required_path(&args[1..], "--stage-dir")?;
            let stage = option(&args[1..], "--stage").ok_or("missing required --stage ID")?;
            let output = required_path(&args[1..], "--output")?;
            let inventory = WorldInventory::build(&stage_dir, &stage)?;
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
                .put_bytes(&bytes, ContentKind::WorldInventory)?;
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
    let stage_dir = required_path(query_args, "--stage-dir")?;
    let stage = option(query_args, "--stage").ok_or("missing required --stage ID")?;
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
    let inventory = WorldInventory::build(&stage_dir, &stage)?;
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
