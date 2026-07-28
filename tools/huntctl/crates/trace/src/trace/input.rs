use super::*;

pub(super) fn decode_pad(bytes: &[u8]) -> Result<RawPadState, TraceError> {
    if bytes[10] & !1 != 0 {
        return Err(TraceError("unknown gameplay trace pad flags".into()));
    }
    Ok(RawPadState {
        buttons: u16_at(bytes, 0),
        stick_x: bytes[2] as i8,
        stick_y: bytes[3] as i8,
        substick_x: bytes[4] as i8,
        substick_y: bytes[5] as i8,
        trigger_left: bytes[6],
        trigger_right: bytes[7],
        analog_a: bytes[8],
        analog_b: bytes[9],
        connected: bytes[10] & 1 != 0,
        error: bytes[11] as i8,
    })
}
