use serde::Serialize;
use std::error::Error;
use std::fmt;

const HEADER_SIZE: usize = 36;
const RECORD_SIZE: usize = 102;
const MAGIC: &[u8; 8] = b"DUSKTRCE";
const PLAYER_PRESENT: u32 = 1 << 0;
const PLAYER_IS_LINK: u32 = 1 << 1;
const EVENT_RUNNING: u32 = 1 << 2;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TraceRecord {
    pub simulation_tick: u64,
    pub tape_frame: Option<u64>,
    pub stage_name: String,
    pub room: i8,
    pub layer: i8,
    pub point: i16,
    pub flags: u32,
    pub player_actor_name: i16,
    pub current_angle_y: i16,
    pub shape_angle_y: i16,
    pub buttons: u16,
    pub stick_x: i8,
    pub stick_y: i8,
    pub position: [f32; 3],
    pub velocity: [f32; 3],
    pub forward_speed: f32,
    pub player_proc_id: Option<u16>,
    pub event_id: i16,
    pub event_mode: u8,
    pub event_status: u8,
    pub event_map_tool_id: u8,
    pub pad_error: i8,
    pub event_name_hash: u32,
    pub nearest_scene_exit_actor_name: Option<i16>,
    pub nearest_scene_exit_position: [f32; 3],
    pub nearest_scene_exit_distance: Option<f32>,
}

impl TraceRecord {
    pub fn player_present(&self) -> bool {
        self.flags & PLAYER_PRESENT != 0
    }

    pub fn player_is_link(&self) -> bool {
        self.flags & PLAYER_IS_LINK != 0
    }

    pub fn event_running(&self) -> bool {
        self.flags & EVENT_RUNNING != 0
    }

    fn location(&self) -> TraceLocation {
        TraceLocation {
            stage_name: self.stage_name.clone(),
            room: self.room,
            point: self.point,
            layer: self.layer,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TraceLocation {
    pub stage_name: String,
    pub room: i8,
    pub point: i16,
    pub layer: i8,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TraceMilestone {
    pub kind: &'static str,
    pub simulation_tick: u64,
    pub tape_frame: Option<u64>,
    pub location: TraceLocation,
    pub position: [f32; 3],
    pub event_id: i16,
    pub event_name_hash: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TraceSummary {
    pub version: u16,
    pub record_count: usize,
    pub capacity_exhausted: bool,
    pub first_playable: Option<TraceMilestone>,
    pub route_control: Option<TraceMilestone>,
    pub first_loading_trigger: Option<TraceMilestone>,
    pub first_loading_transition: Option<TraceMilestone>,
    pub post_load_playable: Option<TraceMilestone>,
    pub first_post_load_event: Option<TraceMilestone>,
    pub intro_cutscene: Option<TraceMilestone>,
    pub final_record: Option<TraceRecord>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DecodedTrace {
    pub version: u16,
    pub capacity_exhausted: bool,
    pub records: Vec<TraceRecord>,
}

#[derive(Debug)]
pub struct TraceError(String);

impl fmt::Display for TraceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Error for TraceError {}

pub fn decode_and_summarize(bytes: &[u8]) -> Result<TraceSummary, TraceError> {
    let decoded = decode(bytes)?;
    Ok(summarize(
        decoded.version,
        decoded.capacity_exhausted,
        decoded.records,
    ))
}

pub fn decode(bytes: &[u8]) -> Result<DecodedTrace, TraceError> {
    if bytes.len() < HEADER_SIZE {
        return Err(TraceError("truncated gameplay trace header".into()));
    }
    if &bytes[..8] != MAGIC {
        return Err(TraceError("bad gameplay trace magic".into()));
    }
    let version = u16::from_le_bytes([bytes[8], bytes[9]]);
    let record_size = u16::from_le_bytes([bytes[10], bytes[11]]) as usize;
    if version != 1 || record_size != RECORD_SIZE {
        return Err(TraceError(format!(
            "unsupported gameplay trace version {version} / record size {record_size}"
        )));
    }
    let declared_count = u64::from_le_bytes(bytes[20..28].try_into().unwrap());
    let count = usize::try_from(declared_count)
        .map_err(|_| TraceError("gameplay trace record count is too large".into()))?;
    let expected = HEADER_SIZE
        .checked_add(
            count
                .checked_mul(RECORD_SIZE)
                .ok_or_else(|| TraceError("gameplay trace size overflow".into()))?,
        )
        .ok_or_else(|| TraceError("gameplay trace size overflow".into()))?;
    if bytes.len() != expected {
        return Err(TraceError(format!(
            "gameplay trace size mismatch: expected {expected}, got {}",
            bytes.len()
        )));
    }

    let capacity_exhausted = u32::from_le_bytes(bytes[28..32].try_into().unwrap()) != 0;
    let records = bytes[HEADER_SIZE..]
        .chunks_exact(RECORD_SIZE)
        .map(decode_record)
        .collect();
    Ok(DecodedTrace {
        version,
        capacity_exhausted,
        records,
    })
}

fn decode_record(bytes: &[u8]) -> TraceRecord {
    let u16_at = |offset| u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap());
    let i16_at = |offset| i16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap());
    let u32_at = |offset| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
    let f32_at = |offset| f32::from_bits(u32_at(offset));
    let raw_frame = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
    let stage_end = bytes[16..24]
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(8);
    let proc_id = u16_at(70);
    let exit_actor_name = i16_at(82);
    let exit_distance = f32_at(96);
    TraceRecord {
        simulation_tick: u64::from_le_bytes(bytes[0..8].try_into().unwrap()),
        tape_frame: (raw_frame != u64::MAX).then_some(raw_frame),
        stage_name: String::from_utf8_lossy(&bytes[16..16 + stage_end]).into_owned(),
        room: bytes[24] as i8,
        layer: bytes[25] as i8,
        point: i16_at(26),
        flags: u32_at(28),
        player_actor_name: i16_at(32),
        current_angle_y: i16_at(34),
        shape_angle_y: i16_at(36),
        buttons: u16_at(38),
        stick_x: bytes[40] as i8,
        stick_y: bytes[41] as i8,
        position: [f32_at(42), f32_at(46), f32_at(50)],
        velocity: [f32_at(54), f32_at(58), f32_at(62)],
        forward_speed: f32_at(66),
        player_proc_id: (proc_id != u16::MAX).then_some(proc_id),
        event_id: i16_at(72),
        event_mode: bytes[74],
        event_status: bytes[75],
        event_map_tool_id: bytes[76],
        pad_error: bytes[77] as i8,
        event_name_hash: u32_at(78),
        nearest_scene_exit_actor_name: (exit_actor_name != -1).then_some(exit_actor_name),
        nearest_scene_exit_position: [f32_at(84), f32_at(88), f32_at(92)],
        nearest_scene_exit_distance: (exit_distance >= 0.0).then_some(exit_distance),
    }
}

fn milestone(kind: &'static str, record: &TraceRecord) -> TraceMilestone {
    TraceMilestone {
        kind,
        simulation_tick: record.simulation_tick,
        tape_frame: record.tape_frame,
        location: record.location(),
        position: record.position,
        event_id: record.event_id,
        event_name_hash: record.event_name_hash,
    }
}

fn summarize(version: u16, capacity_exhausted: bool, records: Vec<TraceRecord>) -> TraceSummary {
    // The title scene also owns a Link actor. The intro route starts only once
    // control is restored at the canonical Ordon Woods spawn.
    let playable_index = records.iter().position(|record| {
        record.stage_name == "F_SP103"
            && record.room == 1
            && record.point == 1
            && record.player_present()
            && record.player_is_link()
            && !record.event_running()
    });
    let loading_index = playable_index.and_then(|index| {
        let initial = records[index].location();
        records[index + 1..]
            .iter()
            .position(|record| record.location() != initial)
            .map(|relative| index + 1 + relative)
    });
    let opening_event_index = playable_index.and_then(|index| {
        records[index + 1..]
            .iter()
            .position(TraceRecord::event_running)
            .map(|relative| index + 1 + relative)
    });
    let route_control_index = opening_event_index.and_then(|index| {
        records[index + 1..]
            .iter()
            .position(|record| {
                record.stage_name == "F_SP103"
                    && record.room == 1
                    && record.player_is_link()
                    && !record.event_running()
            })
            .map(|relative| index + 1 + relative)
    });
    let loading_trigger_index = route_control_index.and_then(|index| {
        records[index + 1..]
            .iter()
            .position(|record| record.stage_name == "F_SP103" && record.event_running())
            .map(|relative| index + 1 + relative)
    });
    let post_load_index = loading_index.and_then(|index| {
        records[index..]
            .iter()
            .position(|record| {
                record.player_present() && record.player_is_link() && !record.event_running()
            })
            .map(|relative| index + relative)
    });
    let post_load_event_index = post_load_index.and_then(|index| {
        records[index + 1..]
            .iter()
            .position(TraceRecord::event_running)
            .map(|relative| index + 1 + relative)
    });
    let intro_cutscene_index = records.iter().position(|record| {
        record.stage_name == "F_SP104"
            && record.point == 26
            && record.event_running()
            && record.event_map_tool_id == 9
    });

    TraceSummary {
        version,
        record_count: records.len(),
        capacity_exhausted,
        first_playable: playable_index.map(|index| milestone("first_playable", &records[index])),
        route_control: route_control_index.map(|index| milestone("route_control", &records[index])),
        first_loading_trigger: loading_trigger_index
            .map(|index| milestone("first_loading_trigger", &records[index])),
        first_loading_transition: loading_index
            .map(|index| milestone("first_loading_transition", &records[index])),
        post_load_playable: post_load_index
            .map(|index| milestone("post_load_playable", &records[index])),
        first_post_load_event: post_load_event_index
            .map(|index| milestone("first_post_load_event", &records[index])),
        intro_cutscene: intro_cutscene_index
            .map(|index| milestone("intro_cutscene", &records[index])),
        final_record: records.last().cloned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_bad_magic() {
        let mut bytes = vec![0; HEADER_SIZE];
        bytes[..8].copy_from_slice(b"NOTTRACE");
        assert!(
            decode_and_summarize(&bytes)
                .unwrap_err()
                .to_string()
                .contains("magic")
        );
    }
}
