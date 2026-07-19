//! Read-only boot choices derived from extracted retail stage resources.

use super::*;
use std::collections::{BTreeMap, BTreeSet};

const STAGE_ROOT: &str = "orig/GZ2E01/files/res/Stage";

pub(super) fn stage_summaries(repository_root: &Path) -> Vec<GraphStageSummary> {
    let root = repository_root.join(STAGE_ROOT);
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut stages = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|stage| valid_stage_id(stage))
        .map(|id| GraphStageSummary {
            friendly_name: friendly_stage_name(&id).map(str::to_owned),
            id,
        })
        .collect::<Vec<_>>();
    stages.sort_by(|left, right| left.id.cmp(&right.id));
    stages
}

pub(super) fn stage_boot_options(
    repository_root: &Path,
    stage: &str,
) -> Result<GraphStageBootOptions, WorkbenchError> {
    if !valid_stage_id(stage) {
        return Err(WorkbenchError::new("stage ID must be 1..=8 uppercase ASCII letters, digits, or underscore"));
    }
    let stage_dir = repository_root.join(STAGE_ROOT).join(stage);
    if !stage_dir.is_dir() {
        return Err(WorkbenchError::new(format!(
            "stage {stage:?} is not present in the extracted retail resources"
        )));
    }

    let mut rooms = BTreeMap::<i8, RoomChoices>::new();
    let entries = fs::read_dir(&stage_dir).map_err(|error| {
        WorkbenchError::new(format!("cannot read {}: {error}", stage_dir.display()))
    })?;
    for entry in entries.filter_map(Result::ok) {
        let Some(room) = entry.file_name().to_str().and_then(room_archive_number) else {
            continue;
        };
        rooms.entry(room).or_default();
    }

    let inventory_path = repository_root
        .join("build/world")
        .join(format!("{stage}.inventory.json"));
    let inventory = fs::read(&inventory_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<CachedInventory>(&bytes).ok());
    if let Some(inventory) = &inventory {
        let mut global_layers = BTreeSet::new();
        for chunk in &inventory.chunks {
            let Some(layer) = layer_from_chunk_tag(&chunk.tag) else {
                continue;
            };
            if let Some(room) = chunk.scope.room {
                rooms.entry(room).or_default().layers.insert(layer);
            } else {
                global_layers.insert(layer);
            }
        }
        for spawn in &inventory.player_spawns {
            let Some(room) = spawn.scope.room else {
                continue;
            };
            let choices = rooms.entry(room).or_default();
            choices.spawn_points.insert(spawn.angle[2]);
            if let Some(layer) = spawn.layer {
                choices.layers.insert(layer);
            }
        }
        for choices in rooms.values_mut() {
            choices.layers.extend(global_layers.iter().copied());
        }
    }

    Ok(GraphStageBootOptions {
        stage: stage.to_owned(),
        friendly_name: friendly_stage_name(stage).map(str::to_owned),
        inventory_indexed: inventory.is_some(),
        rooms: rooms
            .into_iter()
            .map(|(id, mut choices)| {
                choices.layers.insert(-1);
                GraphStageRoomBootOptions {
                    id,
                    spawn_points: choices.spawn_points.into_iter().collect(),
                    layers: choices.layers.into_iter().collect(),
                }
            })
            .collect(),
    })
}

#[derive(Default)]
struct RoomChoices {
    spawn_points: BTreeSet<i16>,
    layers: BTreeSet<i8>,
}

#[derive(Deserialize)]
struct CachedInventory {
    #[serde(default)]
    chunks: Vec<CachedChunk>,
    #[serde(default)]
    player_spawns: Vec<CachedSpawn>,
}

#[derive(Deserialize)]
struct CachedChunk {
    scope: CachedScope,
    tag: String,
}

#[derive(Deserialize)]
struct CachedSpawn {
    scope: CachedScope,
    #[serde(default)]
    layer: Option<i8>,
    angle: [i16; 3],
}

#[derive(Deserialize)]
struct CachedScope {
    #[serde(default)]
    room: Option<i8>,
}

fn room_archive_number(name: &str) -> Option<i8> {
    let digits = name.strip_prefix('R')?.get(..2)?;
    (name.get(3..4) == Some("_") && name.ends_with(".arc"))
        .then(|| digits.parse().ok())
        .flatten()
}

fn layer_from_chunk_tag(tag: &str) -> Option<i8> {
    if tag.len() != 4 || !["ACT", "SCO", "TRE", "Doo"].iter().any(|prefix| tag.starts_with(prefix)) {
        return None;
    }
    match tag.as_bytes()[3] {
        b'0'..=b'9' => Some((tag.as_bytes()[3] - b'0') as i8),
        b'a'..=b'e' => Some((tag.as_bytes()[3] - b'a' + 10) as i8),
        _ => None,
    }
}

fn valid_stage_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 8
        && value
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
}

// Community-readable names for retail identifiers. The identifiers and room
// availability remain grounded in the user's own extracted disc resources.
fn friendly_stage_name(stage: &str) -> Option<&'static str> {
    Some(match stage {
        "D_MN01" => "Lakebed Temple",
        "D_MN01A" => "Lakebed Temple boss",
        "D_MN01B" => "Lakebed Temple miniboss",
        "D_MN04" => "Goron Mines",
        "D_MN04A" => "Goron Mines boss",
        "D_MN04B" => "Goron Mines miniboss",
        "D_MN05" => "Forest Temple",
        "D_MN05A" => "Forest Temple boss",
        "D_MN05B" => "Forest Temple miniboss",
        "D_MN06" => "Temple of Time",
        "D_MN06A" => "Temple of Time boss",
        "D_MN06B" => "Temple of Time miniboss",
        "D_MN07" => "City in the Sky",
        "D_MN07A" => "City in the Sky boss",
        "D_MN07B" => "City in the Sky miniboss",
        "D_MN08" => "Palace of Twilight",
        "D_MN08A" => "Palace of Twilight sub-boss",
        "D_MN08B" => "Palace of Twilight hand room 1",
        "D_MN08C" => "Palace of Twilight hand room 2",
        "D_MN08D" => "Palace of Twilight boss",
        "D_MN09" => "Hyrule Castle",
        "D_MN09A" => "Hyrule Castle boss",
        "D_MN09B" => "Hyrule Field boss",
        "D_MN09C" => "Hyrule Field cutscene",
        "D_MN10" => "Arbiter's Grounds",
        "D_MN10A" => "Arbiter's Grounds boss",
        "D_MN10B" => "Arbiter's Grounds miniboss",
        "D_MN11" => "Snowpeak Ruins",
        "D_MN11A" => "Snowpeak Ruins boss",
        "D_MN11B" => "Snowpeak Ruins miniboss",
        "D_SB00" => "North Hyrule Field ice puzzle",
        "D_SB01" => "Cave of Ordeals",
        "D_SB02" => "South Hyrule Field lantern cave",
        "D_SB03" => "Lake Hylia lantern cave",
        "D_SB04" => "Bridge of Eldin lava cave",
        "D_SB05" => "Small enemy cave 5",
        "D_SB06" => "Small enemy cave 6",
        "D_SB07" => "Small Poe cave",
        "D_SB08" => "Small fire enemy cave",
        "D_SB09" => "Lake Hylia beehive cave",
        "D_SB10" => "Faron Woods tunnel",
        "F_SP00" => "Ordon Ranch",
        "F_SP102" => "Title-screen Hyrule Field",
        "F_SP103" => "Ordon Village",
        "F_SP104" => "Ordon Woods",
        "F_SP108" => "Faron Woods",
        "F_SP109" => "Kakariko Village",
        "F_SP110" => "Death Mountain",
        "F_SP111" => "Kakariko Graveyard",
        "F_SP112" => "Zora's River",
        "F_SP113" => "Zora's Domain",
        "F_SP114" => "Snowpeak",
        "F_SP115" => "Lake Hylia",
        "F_SP116" => "Castle Town",
        "F_SP117" => "Sacred Grove / Temple of Time",
        "F_SP118" => "Arbiter's Grounds exterior",
        "F_SP121" | "F_SP122" | "F_SP123" => "Hyrule Field",
        "F_SP124" => "Gerudo Desert",
        "F_SP125" => "Mirror Chamber",
        "F_SP126" => "Upper Zora's River",
        "F_SP127" => "Faron Woods interior field",
        "F_SP128" => "Hidden Village",
        "F_SP200" => "Hero's Spirit",
        "R_SP01" => "Ordon Village houses",
        "R_SP107" => "Twilight Hyrule Castle",
        "R_SP108" => "Faron Woods house",
        "R_SP109" => "Kakariko houses",
        "R_SP110" => "Death Mountain dojo",
        "R_SP116" => "Castle Town pub / underground",
        "R_SP127" => "Fishing Hole",
        "R_SP128" => "Impa's house",
        "R_SP160" => "Castle Town interiors",
        "R_SP161" => "STAR tent",
        "R_SP209" => "Kakariko ruins",
        "R_SP300" => "Unused interior",
        "R_SP301" => "Unused Hyrule Castle throne room",
        "S_MV000" => "Empty unused stage",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_uses_disc_rooms_and_optional_read_only_inventory_choices() {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("dusklight-stage-catalog-{nonce}"));
        let stage = root.join(STAGE_ROOT).join("F_SP103");
        fs::create_dir_all(&stage).unwrap();
        fs::write(stage.join("R00_00.arc"), []).unwrap();
        fs::write(stage.join("R01_00.arc"), []).unwrap();
        fs::create_dir_all(root.join("build/world")).unwrap();
        fs::write(
            root.join("build/world/F_SP103.inventory.json"),
            br#"{"chunks":[{"scope":{"room":1},"tag":"ACT3"}],"player_spawns":[{"scope":{"room":1},"layer":3,"angle":[0,0,7]}]}"#,
        )
        .unwrap();

        let summaries = stage_summaries(&root);
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].friendly_name.as_deref(), Some("Ordon Village"));
        let options = stage_boot_options(&root, "F_SP103").unwrap();
        assert!(options.inventory_indexed);
        assert_eq!(options.rooms.iter().map(|room| room.id).collect::<Vec<_>>(), [0, 1]);
        assert_eq!(options.rooms[1].spawn_points, [7]);
        assert_eq!(options.rooms[1].layers, [-1, 3]);

        fs::remove_dir_all(root).unwrap();
    }
}
