use super::{NativeChannelStatus, NativeEpisodeShardError, Reader, decode_channel_status};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NativePlayerCollisionSolverWall {
    pub flags: u32,
    pub angle_y: i16,
    pub wall_radius_squared: f32,
    pub wall_height: f32,
    pub wall_radius: f32,
    pub direct_wall_height: f32,
    pub realized_center: [f32; 3],
    pub realized_radius: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NativePlayerCollisionSolverObservation {
    pub flags: u32,
    pub wall_table_size: i32,
    pub water_mode: u8,
    pub line_start: [f32; 3],
    pub line_end: [f32; 3],
    pub wall_cylinder_center: [f32; 3],
    pub wall_cylinder_radius: f32,
    pub wall_cylinder_height: f32,
    pub ground_check_offset: f32,
    pub roof_correction_height: f32,
    pub water_check_offset: f32,
    pub wall_circles: [NativePlayerCollisionSolverWall; 3],
}

pub(super) fn decode_player_collision_solver(
    reader: &mut Reader<'_>,
) -> Result<(NativeChannelStatus, NativePlayerCollisionSolverObservation), NativeEpisodeShardError>
{
    let status = decode_channel_status(reader)?;
    if reader.u8()? != 3 || reader.u16()? != 0 {
        return Err(NativeEpisodeShardError::new(
            "invalid player-collision-solver wall count or reserved bytes",
        ));
    }
    let flags = reader.u32()?;
    if flags & !0x00f1_fffe != 0 {
        return Err(NativeEpisodeShardError::new(
            "player collision solver has unknown flags",
        ));
    }
    let wall_table_size = reader.i32()?;
    let water_mode = reader.u8()?;
    if reader.u8()? != 0 || reader.u16()? != 0 {
        return Err(NativeEpisodeShardError::new(
            "nonzero player-collision-solver reserved bytes",
        ));
    }
    let line_start = reader.f32x3()?;
    let line_end = reader.f32x3()?;
    let wall_cylinder_center = reader.f32x3()?;
    let wall_cylinder_radius = reader.f32()?;
    let wall_cylinder_height = reader.f32()?;
    let ground_check_offset = reader.f32()?;
    let roof_correction_height = reader.f32()?;
    let water_check_offset = reader.f32()?;
    let mut walls = Vec::with_capacity(3);
    for _ in 0..3 {
        let wall_flags = reader.u32()?;
        if wall_flags & !0x6 != 0 {
            return Err(NativeEpisodeShardError::new(
                "player collision solver wall has unknown flags",
            ));
        }
        let angle_y = reader.i16()?;
        if reader.u16()? != 0 {
            return Err(NativeEpisodeShardError::new(
                "nonzero player-collision-solver wall reserved bytes",
            ));
        }
        walls.push(NativePlayerCollisionSolverWall {
            flags: wall_flags,
            angle_y,
            wall_radius_squared: reader.f32()?,
            wall_height: reader.f32()?,
            wall_radius: reader.f32()?,
            direct_wall_height: reader.f32()?,
            realized_center: reader.f32x3()?,
            realized_radius: reader.f32()?,
        });
    }
    let solver = NativePlayerCollisionSolverObservation {
        flags,
        wall_table_size,
        water_mode,
        line_start,
        line_end,
        wall_cylinder_center,
        wall_cylinder_radius,
        wall_cylinder_height,
        ground_check_offset,
        roof_correction_height,
        water_check_offset,
        wall_circles: walls
            .try_into()
            .expect("exact player-collision-solver wall count"),
    };
    if status != NativeChannelStatus::Present && solver != Default::default() {
        return Err(NativeEpisodeShardError::new(
            "player-collision-solver payload is present for an unavailable channel",
        ));
    }
    Ok((status, solver))
}
