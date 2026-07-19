use super::*;

pub(super) fn decode_name(bytes: &[u8]) -> Result<String, TraceError> {
    let end = bytes
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(bytes.len());
    if bytes[end..].iter().any(|value| *value != 0)
        || bytes[..end].iter().any(|value| !value.is_ascii_graphic())
    {
        return Err(TraceError("invalid canonical gameplay trace name".into()));
    }
    Ok(String::from_utf8(bytes[..end].to_vec()).expect("validated ASCII"))
}

pub(super) fn checked_region_end(
    start: usize,
    count: usize,
    stride: usize,
) -> Result<usize, TraceError> {
    start
        .checked_add(
            count
                .checked_mul(stride)
                .ok_or_else(|| TraceError("gameplay trace size overflow".into()))?,
        )
        .ok_or_else(|| TraceError("gameplay trace size overflow".into()))
}

pub(super) fn count_at(bytes: &[u8], offset: usize) -> Result<usize, TraceError> {
    let count = usize::try_from(u64_at(bytes, offset))
        .map_err(|_| TraceError("gameplay trace record count is too large".into()))?;
    if count > MAX_TRACE_RECORDS {
        return Err(TraceError(format!(
            "gameplay trace record count exceeds {MAX_TRACE_RECORDS}"
        )));
    }
    Ok(count)
}

pub(super) fn usize_at_u64(bytes: &[u8], offset: usize) -> Result<usize, TraceError> {
    usize::try_from(u64_at(bytes, offset))
        .map_err(|_| TraceError("gameplay trace offset is too large".into()))
}

pub(super) fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().expect("bounded field"))
}

pub(super) fn i16_at(bytes: &[u8], offset: usize) -> i16 {
    i16::from_le_bytes(bytes[offset..offset + 2].try_into().expect("bounded field"))
}

pub(super) fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("bounded field"))
}

pub(super) fn i32_at(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("bounded field"))
}

pub(super) fn u64_at(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().expect("bounded field"))
}

pub(super) fn f32_at(bytes: &[u8], offset: usize) -> f32 {
    f32::from_bits(u32_at(bytes, offset))
}
