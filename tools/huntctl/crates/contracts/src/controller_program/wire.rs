use super::{BLEND_ADD, BLEND_REPLACE, ControllerError, CoordinateFrame, StickBlend};

pub(super) fn require_zero(
    index: usize,
    input: &[u8],
    start: usize,
) -> Result<(), ControllerError> {
    if input[start..].iter().any(|byte| *byte != 0) {
        Err(ControllerError::new(format!(
            "layer {index} has nonzero reserved payload bytes"
        )))
    } else {
        Ok(())
    }
}

pub(super) fn encode_blend(blend: StickBlend) -> u8 {
    match blend {
        StickBlend::Replace => BLEND_REPLACE,
        StickBlend::Add => BLEND_ADD,
    }
}

pub(super) fn decode_stick_blend(index: usize, value: u8) -> Result<StickBlend, ControllerError> {
    match value {
        BLEND_REPLACE => Ok(StickBlend::Replace),
        BLEND_ADD => Ok(StickBlend::Add),
        _ => Err(ControllerError::new(format!(
            "stick layer {index} has invalid blend {value}"
        ))),
    }
}

pub(super) fn encode_coordinate_frame(frame: CoordinateFrame) -> u8 {
    match frame {
        CoordinateFrame::World => 0,
        CoordinateFrame::Player => 1,
        CoordinateFrame::Camera => 2,
    }
}

pub(super) fn decode_coordinate_frame(
    index: usize,
    value: u8,
) -> Result<CoordinateFrame, ControllerError> {
    match value {
        0 => Ok(CoordinateFrame::World),
        1 => Ok(CoordinateFrame::Player),
        2 => Ok(CoordinateFrame::Camera),
        _ => Err(ControllerError::new(format!(
            "layer {index} has invalid coordinate frame {value}"
        ))),
    }
}

pub(super) fn put_u16(output: &mut [u8], offset: usize, value: u16) {
    output[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

pub(super) fn put_i16(output: &mut [u8], offset: usize, value: i16) {
    output[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

pub(super) fn put_u32(output: &mut [u8], offset: usize, value: u32) {
    output[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

pub(super) fn put_u64(output: &mut [u8], offset: usize, value: u64) {
    output[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

pub(super) fn put_f32(output: &mut [u8], offset: usize, value: f32) {
    output[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

pub(super) fn get_u16(input: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(input[offset..offset + 2].try_into().expect("u16 slice"))
}

pub(super) fn get_i16(input: &[u8], offset: usize) -> i16 {
    i16::from_le_bytes(input[offset..offset + 2].try_into().expect("i16 slice"))
}

pub(super) fn get_u32(input: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(input[offset..offset + 4].try_into().expect("u32 slice"))
}

pub(super) fn get_u64(input: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(input[offset..offset + 8].try_into().expect("u64 slice"))
}

pub(super) fn get_f32(input: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes(input[offset..offset + 4].try_into().expect("f32 slice"))
}
