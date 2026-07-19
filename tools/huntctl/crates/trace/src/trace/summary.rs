use super::*;

pub(super) fn validate_tick_rate(bytes: &[u8]) -> Result<(), TraceError> {
    if u32_at(bytes, 12) == 0 || u32_at(bytes, 16) == 0 {
        return Err(TraceError("invalid gameplay trace tick rate".into()));
    }
    Ok(())
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
        event_name_hash_present: record.event_name_hash_present,
    }
}

pub(super) fn summarize(decoded: DecodedTrace) -> TraceSummary {
    let boot = decoded.boot.clone();
    let records = decoded.records;
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
    let requested_channels = TraceChannel::ALL
        .into_iter()
        .filter(|channel| decoded.requested_channels & channel.bit() != 0)
        .map(TraceChannel::name)
        .collect();

    TraceSummary {
        version: decoded.version,
        boot,
        requested_channels,
        record_count: records.len(),
        capacity_exhausted: decoded.capacity_exhausted,
        retention: decoded.retention,
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
