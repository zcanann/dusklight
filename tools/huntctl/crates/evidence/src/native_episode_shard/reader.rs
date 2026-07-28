//! Bounds-checked primitive decoding for native episode payload bytes.

use super::*;

pub(super) fn decode_actor_identity(
    reader: &mut Reader<'_>,
) -> Result<NativeActorIdentity, NativeEpisodeShardError> {
    let present = reader.bool()?;
    let runtime_generation = reader.u32()?;
    let actor_name = reader.i16()?;
    let set_id = reader.u16()?;
    let home_room = reader.i8()?;
    let current_room = reader.i8()?;
    let home_present = reader.bool()?;
    if reader.u8()? != 0 {
        return Err(NativeEpisodeShardError::new(
            "nonzero actor-identity reserved byte",
        ));
    }
    let position = reader.f32x3()?;
    if present != home_present {
        return Err(NativeEpisodeShardError::new(
            "actor identity has inconsistent presence",
        ));
    }
    Ok(NativeActorIdentity {
        present,
        runtime_generation,
        actor_name,
        set_id,
        home_room,
        current_room,
        home_position: home_present.then_some(position),
    })
}

pub(super) fn decode_pad(reader: &mut Reader<'_>) -> Result<NativeRawPad, NativeEpisodeShardError> {
    let start = reader.offset;
    let buttons = reader.u16()?;
    let stick_x = reader.i8()?;
    let stick_y = reader.i8()?;
    let substick_x = reader.i8()?;
    let substick_y = reader.i8()?;
    let trigger_left = reader.u8()?;
    let trigger_right = reader.u8()?;
    let analog_a = reader.u8()?;
    let analog_b = reader.u8()?;
    let connection = reader.u8()?;
    let connected = match connection {
        0 => false,
        1 => true,
        _ => {
            let wire = reader.bytes.get(start..start + 12).unwrap_or_default();
            return Err(NativeEpisodeShardError::new(format!(
                "invalid raw PAD flags {connection:#04x} at payload offset {} (wire={wire:02x?})",
                start + 10,
            )));
        }
    };
    let pad = NativeRawPad {
        buttons,
        stick_x,
        stick_y,
        substick_x,
        substick_y,
        trigger_left,
        trigger_right,
        analog_a,
        analog_b,
        connected,
        error: reader.i8()?,
    };
    Ok(pad)
}

pub(super) struct Reader<'a> {
    bytes: &'a [u8],
    pub(super) offset: usize,
}

impl<'a> Reader<'a> {
    pub(super) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }
    pub(super) fn done(&self) -> bool {
        self.offset == self.bytes.len()
    }
    pub(super) fn bytes(&mut self, count: usize) -> Result<&'a [u8], NativeEpisodeShardError> {
        let end = self
            .offset
            .checked_add(count)
            .ok_or_else(|| NativeEpisodeShardError::new("native episode offset overflow"))?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| NativeEpisodeShardError::new("truncated native episode shard"))?;
        self.offset = end;
        Ok(value)
    }
    pub(super) fn u8(&mut self) -> Result<u8, NativeEpisodeShardError> {
        Ok(self.bytes(1)?[0])
    }
    pub(super) fn i8(&mut self) -> Result<i8, NativeEpisodeShardError> {
        Ok(self.u8()? as i8)
    }
    pub(super) fn bool(&mut self) -> Result<bool, NativeEpisodeShardError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(NativeEpisodeShardError::new("noncanonical boolean")),
        }
    }
    pub(super) fn u16(&mut self) -> Result<u16, NativeEpisodeShardError> {
        Ok(u16::from_le_bytes(
            self.bytes(2)?.try_into().expect("exact length"),
        ))
    }
    pub(super) fn i16(&mut self) -> Result<i16, NativeEpisodeShardError> {
        Ok(i16::from_le_bytes(
            self.bytes(2)?.try_into().expect("exact length"),
        ))
    }
    pub(super) fn u32(&mut self) -> Result<u32, NativeEpisodeShardError> {
        Ok(u32::from_le_bytes(
            self.bytes(4)?.try_into().expect("exact length"),
        ))
    }
    pub(super) fn i32(&mut self) -> Result<i32, NativeEpisodeShardError> {
        Ok(i32::from_le_bytes(
            self.bytes(4)?.try_into().expect("exact length"),
        ))
    }
    pub(super) fn u64(&mut self) -> Result<u64, NativeEpisodeShardError> {
        Ok(u64::from_le_bytes(
            self.bytes(8)?.try_into().expect("exact length"),
        ))
    }
    pub(super) fn usize_u64(&mut self) -> Result<usize, NativeEpisodeShardError> {
        usize::try_from(self.u64()?)
            .map_err(|_| NativeEpisodeShardError::new("native episode size overflow"))
    }
    pub(super) fn f32(&mut self) -> Result<f32, NativeEpisodeShardError> {
        let value = f32::from_bits(self.u32()?);
        if !value.is_finite() || (value == 0.0 && value.is_sign_negative()) {
            return Err(NativeEpisodeShardError::new(
                "noncanonical observation float",
            ));
        }
        Ok(value)
    }
    pub(super) fn f32x3(&mut self) -> Result<[f32; 3], NativeEpisodeShardError> {
        Ok([self.f32()?, self.f32()?, self.f32()?])
    }
    pub(super) fn f32x4(&mut self) -> Result<[f32; 4], NativeEpisodeShardError> {
        Ok([self.f32()?, self.f32()?, self.f32()?, self.f32()?])
    }
    pub(super) fn i16x3(&mut self) -> Result<[i16; 3], NativeEpisodeShardError> {
        Ok([self.i16()?, self.i16()?, self.i16()?])
    }
    pub(super) fn fixed_name(&mut self) -> Result<String, NativeEpisodeShardError> {
        self.fixed_string(8)
    }
    pub(super) fn fixed_string(&mut self, count: usize) -> Result<String, NativeEpisodeShardError> {
        let bytes = self.bytes(count)?;
        let end = bytes
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(bytes.len());
        if bytes[end..].iter().any(|byte| *byte != 0) {
            return Err(NativeEpisodeShardError::new("noncanonical fixed string"));
        }
        std::str::from_utf8(&bytes[..end])
            .map(str::to_owned)
            .map_err(|_| NativeEpisodeShardError::new("fixed string is not UTF-8"))
    }
    pub(super) fn string16(&mut self) -> Result<String, NativeEpisodeShardError> {
        let count = usize::from(self.u16()?);
        std::str::from_utf8(self.bytes(count)?)
            .map(str::to_owned)
            .map_err(|_| NativeEpisodeShardError::new("metadata string is not UTF-8"))
    }
    pub(super) fn vec(&mut self, count: usize) -> Result<Vec<u8>, NativeEpisodeShardError> {
        Ok(self.bytes(count)?.to_vec())
    }
}
