//! Decode player resource channels and their availability status.

use super::*;

pub(super) fn decode_channel_status(
    reader: &mut Reader<'_>,
) -> Result<NativeChannelStatus, NativeEpisodeShardError> {
    match reader.u8()? {
        0 => Ok(NativeChannelStatus::NotSampled),
        1 => Ok(NativeChannelStatus::Present),
        2 => Ok(NativeChannelStatus::Absent),
        3 => Ok(NativeChannelStatus::Unavailable),
        _ => Err(NativeEpisodeShardError::new(
            "invalid collision channel status",
        )),
    }
}

pub(super) fn decode_player_resources(
    reader: &mut Reader<'_>,
) -> Result<(NativeChannelStatus, NativePlayerResourcesObservation), NativeEpisodeShardError> {
    let status = decode_channel_status(reader)?;
    if reader.u8()? != 0 {
        return Err(NativeEpisodeShardError::new(
            "nonzero player-resources reserved byte",
        ));
    }
    let maximum_life = reader.u16()?;
    let life = reader.u16()?;
    let rupees = reader.u16()?;
    let rupee_capacity = reader.u16()?;
    let maximum_oil = reader.u16()?;
    let oil = reader.u16()?;
    let maximum_magic = reader.u8()?;
    let magic = reader.u8()?;
    let wallet = reader.u8()?;
    let transform_status = reader.u8()?;
    let world_time = reader.f32()?;
    let date = reader.u16()?;
    let arrows = reader.u8()?;
    let arrow_capacity = reader.u8()?;
    let pachinko = reader.u8()?;
    let poe_souls = reader.u8()?;
    let small_keys = reader.u8()?;
    let dungeon_items = reader.u8()?;
    if dungeon_items & !0x0f != 0 || reader.u16()? != 0 {
        return Err(NativeEpisodeShardError::new(
            "invalid player-resources flags or reserved bytes",
        ));
    }
    let resources = NativePlayerResourcesObservation {
        maximum_life,
        life,
        rupees,
        rupee_capacity,
        maximum_oil,
        oil,
        maximum_magic,
        magic,
        wallet,
        transform_status,
        world_time,
        date,
        arrows,
        arrow_capacity,
        pachinko,
        poe_souls,
        small_keys,
        dungeon_map: dungeon_items & (1 << 0) != 0,
        dungeon_compass: dungeon_items & (1 << 1) != 0,
        dungeon_boss_key: dungeon_items & (1 << 2) != 0,
        dungeon_warp: dungeon_items & (1 << 3) != 0,
        inventory: reader
            .bytes(24)?
            .try_into()
            .expect("exact inventory length"),
        selected_items: reader
            .bytes(4)?
            .try_into()
            .expect("exact selected-item length"),
        mixed_items: reader
            .bytes(4)?
            .try_into()
            .expect("exact mixed-item length"),
        equipment: reader.bytes(6)?.try_into().expect("exact equipment length"),
        bomb_counts: reader
            .bytes(3)?
            .try_into()
            .expect("exact bomb-count length"),
        bomb_capacities: reader
            .bytes(3)?
            .try_into()
            .expect("exact bomb-capacity length"),
        bottle_quantities: reader
            .bytes(4)?
            .try_into()
            .expect("exact bottle-quantity length"),
        acquired_item_bits: reader
            .bytes(32)?
            .try_into()
            .expect("exact acquired-item length"),
        collect_item_bits: reader
            .bytes(8)?
            .try_into()
            .expect("exact collect-item length"),
        collected_crystal_bits: reader.u8()?,
        collected_mirror_bits: reader.u8()?,
    };
    if status != NativeChannelStatus::Present
        && resources != NativePlayerResourcesObservation::default()
    {
        return Err(NativeEpisodeShardError::new(
            "player-resources payload is present for an unavailable channel",
        ));
    }
    Ok((status, resources))
}

pub(super) type PlannerRuntimeChannels = (
    NativeChannelStatus,
    Option<NativeRuntimeFileObservation>,
    NativeChannelStatus,
    Option<NativeReturnPlaceObservation>,
    NativeChannelStatus,
    Option<NativeRestartObservation>,
    NativeChannelStatus,
    Option<NativeEventHandoffObservation>,
);
