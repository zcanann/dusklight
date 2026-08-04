//! Compact, versioned request transport for repeated persistent suffix batches.
//!
//! Authored launch requests remain JSON. Once a native process has authenticated
//! that source, tactic exploration uses this bounded envelope so the hot path
//! carries raw content identities, controller bytes, and RLE PAD states rather
//! than repeating JSON field names and hexadecimal blobs.

use dusklight_search::search::MacroAction;
use dusklight_search::suffix_batch::{
    NATIVE_CACHED_SUFFIX_BATCH_SCHEMA, NATIVE_VARIABLE_CACHED_SUFFIX_BATCH_SCHEMA,
    NativeSuffixBatch,
};
use std::error::Error;
use std::fmt;
use xxhash_rust::xxh3::xxh3_128;

pub const COMPACT_SUFFIX_BATCH_MAGIC: [u8; 8] = *b"DSKSBX\x02\0";
const HEADER_BYTES: usize = 28;
const FLAG_VERIFY_STATE_HASHES: u8 = 1 << 0;
const FLAG_CHECKPOINT_CACHE: u8 = 1 << 1;
const FLAG_SOURCE_IDENTITY: u8 = 1 << 2;
const FLAG_RETAIN_CANDIDATE_CHECKPOINTS: u8 = 1 << 3;
const FLAG_RETAIN_LIVE_ENDPOINT: u8 = 1 << 4;
const FLAG_VARIABLE_CANDIDATE_TICKS: u8 = 1 << 5;
const FLAG_RETAIN_CANDIDATE_INDEX: u8 = 1 << 6;
const FLAG_CANDIDATE_CANCELLATION_GUARDS: u8 = 1 << 7;
const RECORDED_REPLAY_WINDOW: u8 = 2;
const MAXIMUM_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Eq, PartialEq)]
pub struct CompactSuffixBatchError(String);

impl CompactSuffixBatchError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for CompactSuffixBatchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for CompactSuffixBatchError {}

/// Encodes the cached PAD/controller subset used by tactic exploration.
///
/// The native parser applies the same semantic bounds as the JSON parser.
/// Rust still retains the complete `NativeSuffixBatch` and uses it to attach
/// the returned result, so this is a transport representation rather than a
/// second authority for the request.
pub fn encode_compact_suffix_batch(
    batch: &NativeSuffixBatch,
) -> Result<Vec<u8>, CompactSuffixBatchError> {
    if !matches!(
        batch.schema.as_str(),
        NATIVE_CACHED_SUFFIX_BATCH_SCHEMA | NATIVE_VARIABLE_CACHED_SUFFIX_BATCH_SCHEMA
    ) {
        return Err(CompactSuffixBatchError::new(
            "compact suffix transport requires a cached schema",
        ));
    }
    let variable_candidates = batch.schema == NATIVE_VARIABLE_CACHED_SUFFIX_BATCH_SCHEMA;
    let cache = batch
        .checkpoint_cache
        .as_ref()
        .ok_or_else(|| CompactSuffixBatchError::new("compact suffix transport requires a cache"))?;
    if batch.checkpoint_validation.kind != "recorded_replay_window" {
        return Err(CompactSuffixBatchError::new(
            "compact suffix transport requires recorded replay validation",
        ));
    }

    let source_frame = u64::try_from(batch.source_frame)
        .map_err(|_| CompactSuffixBatchError::new("source frame does not fit u64"))?;
    let maximum_ticks = u16::try_from(batch.maximum_ticks)
        .map_err(|_| CompactSuffixBatchError::new("maximum ticks does not fit u16"))?;
    if maximum_ticks == 0 || maximum_ticks > 4_096 {
        return Err(CompactSuffixBatchError::new(
            "maximum ticks is outside the compact transport bound",
        ));
    }
    let validation_ticks = u16::try_from(batch.checkpoint_validation.ticks)
        .map_err(|_| CompactSuffixBatchError::new("validation ticks does not fit u16"))?;
    if validation_ticks == 0 || validation_ticks > 256 {
        return Err(CompactSuffixBatchError::new(
            "validation ticks is outside the compact transport bound",
        ));
    }
    let capacity_bytes = u32::try_from(cache.capacity_bytes)
        .map_err(|_| CompactSuffixBatchError::new("cache byte capacity does not fit u32"))?;
    let capacity_entries = u8::try_from(cache.capacity_entries)
        .map_err(|_| CompactSuffixBatchError::new("cache entry capacity does not fit u8"))?;
    let source_route_ticks = u32::try_from(cache.source_route_ticks)
        .map_err(|_| CompactSuffixBatchError::new("source route ticks does not fit u32"))?;
    let candidate_count = u16::try_from(batch.candidates.len())
        .map_err(|_| CompactSuffixBatchError::new("candidate count does not fit u16"))?;
    if candidate_count == 0 {
        return Err(CompactSuffixBatchError::new(
            "compact suffix batch has no candidates",
        ));
    }

    let boundary = decode_identity(
        &batch.source_boundary_fingerprint,
        "source boundary fingerprint",
    )?;
    let source_identity = cache
        .source_identity
        .as_deref()
        .map(|value| decode_identity(value, "checkpoint source identity"))
        .transpose()?;

    let mut flags = FLAG_CHECKPOINT_CACHE;
    if batch.verify_state_hashes {
        flags |= FLAG_VERIFY_STATE_HASHES;
    }
    if source_identity.is_some() {
        flags |= FLAG_SOURCE_IDENTITY;
    }
    if cache.retain_candidate_checkpoints {
        flags |= FLAG_RETAIN_CANDIDATE_CHECKPOINTS;
    }
    if cache.retain_live_endpoint {
        flags |= FLAG_RETAIN_LIVE_ENDPOINT;
    }
    if variable_candidates {
        flags |= FLAG_VARIABLE_CANDIDATE_TICKS;
    }
    if cache.retain_candidate_index.is_some() {
        flags |= FLAG_RETAIN_CANDIDATE_INDEX;
    }
    if batch
        .candidates
        .iter()
        .any(|candidate| candidate.cancellation_guard.is_some())
    {
        flags |= FLAG_CANDIDATE_CANCELLATION_GUARDS;
    }
    if usize::from(cache.retain_candidate_checkpoints)
        + usize::from(cache.retain_live_endpoint)
        + usize::from(cache.retain_candidate_index.is_some())
        > 1
    {
        return Err(CompactSuffixBatchError::new(
            "compact suffix batch requests conflicting checkpoint retention",
        ));
    }
    if cache
        .retain_candidate_index
        .is_some_and(|index| !variable_candidates || index >= batch.candidates.len())
    {
        return Err(CompactSuffixBatchError::new(
            "compact suffix retained candidate index is invalid",
        ));
    }

    let mut payload = Vec::with_capacity(128 + batch.candidates.len() * 96);
    payload.push(flags);
    payload.extend_from_slice(&source_frame.to_le_bytes());
    payload.extend_from_slice(&boundary);
    payload.push(RECORDED_REPLAY_WINDOW);
    payload.extend_from_slice(&validation_ticks.to_le_bytes());
    payload.extend_from_slice(&maximum_ticks.to_le_bytes());
    payload.extend_from_slice(&capacity_bytes.to_le_bytes());
    payload.push(capacity_entries);
    payload.extend_from_slice(&source_route_ticks.to_le_bytes());
    if let Some(identity) = source_identity {
        payload.extend_from_slice(&identity);
    }
    payload.extend_from_slice(&candidate_count.to_le_bytes());
    if let Some(index) = cache.retain_candidate_index {
        let index = u16::try_from(index)
            .map_err(|_| CompactSuffixBatchError::new("retained candidate index exceeds u16"))?;
        payload.extend_from_slice(&index.to_le_bytes());
    }

    for candidate in &batch.candidates {
        let id = candidate.id.as_bytes();
        let id_length = u8::try_from(id.len())
            .map_err(|_| CompactSuffixBatchError::new("candidate id exceeds 255 bytes"))?;
        if id.is_empty() || id.len() > 128 || !id.iter().all(|byte| (0x21..=0x7e).contains(byte)) {
            return Err(CompactSuffixBatchError::new(
                "candidate id is not canonical printable ASCII",
            ));
        }
        payload.push(id_length);
        payload.extend_from_slice(id);

        let candidate_maximum_ticks = candidate.maximum_ticks.unwrap_or(batch.maximum_ticks);
        if candidate_maximum_ticks == 0
            || candidate_maximum_ticks > batch.maximum_ticks
            || variable_candidates != candidate.maximum_ticks.is_some()
        {
            return Err(CompactSuffixBatchError::new(
                "candidate maximum ticks differs from its compact schema",
            ));
        }
        if variable_candidates {
            let candidate_maximum_ticks = u16::try_from(candidate_maximum_ticks)
                .map_err(|_| CompactSuffixBatchError::new("candidate maximum ticks exceeds u16"))?;
            payload.extend_from_slice(&candidate_maximum_ticks.to_le_bytes());
        }

        if flags & FLAG_CANDIDATE_CANCELLATION_GUARDS != 0 {
            let cells = candidate
                .cancellation_guard
                .as_ref()
                .map(|guard| guard.allowed_stage_rooms.as_slice())
                .unwrap_or_default();
            let cell_count = u8::try_from(cells.len()).map_err(|_| {
                CompactSuffixBatchError::new("candidate cancellation guard exceeds 255 cells")
            })?;
            payload.push(cell_count);
            for (index, cell) in cells.iter().enumerate() {
                if cell.stage.is_empty()
                    || cell.stage.len() > 8
                    || !cell.stage.is_ascii()
                    || index > 0 && &cells[index - 1] >= cell
                {
                    return Err(CompactSuffixBatchError::new(
                        "candidate cancellation guard is not canonical",
                    ));
                }
                payload.push(cell.room as u8);
                payload.push(cell.stage.len() as u8);
                payload.extend_from_slice(cell.stage.as_bytes());
            }
        }

        let controller = candidate
            .controller_program_hex
            .as_deref()
            .map(decode_lower_hex)
            .transpose()?
            .unwrap_or_default();
        let controller_length = u16::try_from(controller.len())
            .map_err(|_| CompactSuffixBatchError::new("controller program exceeds 65535 bytes"))?;
        payload.extend_from_slice(&controller_length.to_le_bytes());
        payload.extend_from_slice(&controller);

        let run_count = u16::try_from(candidate.actions.len())
            .map_err(|_| CompactSuffixBatchError::new("PAD run count exceeds 65535"))?;
        payload.extend_from_slice(&run_count.to_le_bytes());
        for action in &candidate.actions {
            let MacroAction::PadRun {
                pad,
                frames,
                imported_owned_ports,
                port_one_secondary_pads,
            } = action
            else {
                return Err(CompactSuffixBatchError::new(
                    "compact suffix candidate contains a non-PAD action",
                ));
            };
            if imported_owned_ports.is_some() || port_one_secondary_pads.is_some() {
                return Err(CompactSuffixBatchError::new(
                    "compact suffix candidate contains noncanonical secondary ports",
                ));
            }
            let frames = u16::try_from(*frames)
                .map_err(|_| CompactSuffixBatchError::new("PAD run duration exceeds 65535"))?;
            if frames == 0 {
                return Err(CompactSuffixBatchError::new("PAD run duration is zero"));
            }
            payload.extend_from_slice(&frames.to_le_bytes());
            payload.extend_from_slice(&pad.buttons.to_le_bytes());
            payload.push(pad.stick_x as u8);
            payload.push(pad.stick_y as u8);
            payload.push(pad.substick_x as u8);
            payload.push(pad.substick_y as u8);
            payload.push(pad.trigger_left);
            payload.push(pad.trigger_right);
            payload.push(pad.analog_a);
            payload.push(pad.analog_b);
            payload.push(u8::from(pad.connected));
            payload.push(pad.error as u8);
        }
    }
    if payload.len() > MAXIMUM_BYTES - HEADER_BYTES {
        return Err(CompactSuffixBatchError::new(
            "compact suffix payload exceeds 64 MiB",
        ));
    }

    let payload_length = u32::try_from(payload.len())
        .map_err(|_| CompactSuffixBatchError::new("compact suffix payload exceeds u32"))?;
    let mut output = Vec::with_capacity(HEADER_BYTES + payload.len());
    output.extend_from_slice(&COMPACT_SUFFIX_BATCH_MAGIC);
    output.extend_from_slice(&payload_length.to_le_bytes());
    output.extend_from_slice(&xxh3_128(&payload).to_be_bytes());
    output.extend_from_slice(&payload);
    Ok(output)
}

fn decode_identity(value: &str, label: &str) -> Result<[u8; 16], CompactSuffixBatchError> {
    let bytes = decode_lower_hex(value)?;
    bytes.try_into().map_err(|_| {
        CompactSuffixBatchError::new(format!("{label} is not a 128-bit lowercase identity"))
    })
}

fn decode_lower_hex(value: &str) -> Result<Vec<u8>, CompactSuffixBatchError> {
    if value.is_empty() || value.len() & 1 != 0 {
        return Err(CompactSuffixBatchError::new(
            "hex value is empty or has odd length",
        ));
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let high = lower_hex_nibble(pair[0])?;
            let low = lower_hex_nibble(pair[1])?;
            Ok((high << 4) | low)
        })
        .collect()
}

fn lower_hex_nibble(byte: u8) -> Result<u8, CompactSuffixBatchError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(CompactSuffixBatchError::new(
            "hex value is not canonical lowercase hexadecimal",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dusklight_search::search::SearchPadState;
    use dusklight_search::suffix_batch::{
        NativeCheckpointCacheRequest, NativeCheckpointValidation, NativeSuffixCancellationGuard,
        NativeSuffixCandidate, NativeSuffixStageRoom,
    };

    fn fixture() -> NativeSuffixBatch {
        NativeSuffixBatch {
            schema: NATIVE_CACHED_SUFFIX_BATCH_SCHEMA.into(),
            source_frame: 506,
            source_boundary_fingerprint: "0123456789abcdef0123456789abcdef".into(),
            checkpoint_validation: NativeCheckpointValidation {
                kind: "recorded_replay_window".into(),
                ticks: 8,
            },
            maximum_ticks: 4,
            verify_state_hashes: false,
            checkpoint_cache: Some(NativeCheckpointCacheRequest {
                capacity_bytes: 640 * 1024 * 1024,
                capacity_entries: 2,
                source_identity: Some("fedcba9876543210fedcba9876543210".into()),
                source_route_ticks: 40,
                retain_candidate_checkpoints: true,
                retain_live_endpoint: false,
                retain_candidate_index: None,
            }),
            candidates: vec![NativeSuffixCandidate {
                id: "candidate-1".into(),
                actions: vec![MacroAction::PadRun {
                    pad: SearchPadState {
                        buttons: 0x100,
                        stick_x: -4,
                        stick_y: 127,
                        substick_x: 0,
                        substick_y: 1,
                        trigger_left: 2,
                        trigger_right: 3,
                        analog_a: 4,
                        analog_b: 5,
                        connected: true,
                        error: -1,
                    },
                    frames: 4,
                    imported_owned_ports: None,
                    port_one_secondary_pads: None,
                }],
                controller_program_hex: None,
                maximum_ticks: None,
                cancellation_guard: None,
            }],
        }
    }

    #[test]
    fn compact_envelope_is_canonical_and_materially_smaller_than_json() {
        let batch = fixture();
        let encoded = encode_compact_suffix_batch(&batch).unwrap();
        let json = serde_json::to_vec_pretty(&batch).unwrap();
        assert_eq!(&encoded[..8], &COMPACT_SUFFIX_BATCH_MAGIC);
        assert_eq!(
            u32::from_le_bytes(encoded[8..12].try_into().unwrap()) as usize,
            encoded.len() - HEADER_BYTES
        );
        assert_eq!(
            &encoded[12..28],
            &xxh3_128(&encoded[HEADER_BYTES..]).to_be_bytes()
        );
        assert!(encoded.len() * 4 < json.len());
    }

    #[test]
    fn compact_envelope_rejects_non_pad_actions() {
        let mut batch = fixture();
        batch.candidates[0].actions = vec![MacroAction::Neutral { frames: 4 }];
        assert!(
            encode_compact_suffix_batch(&batch)
                .unwrap_err()
                .to_string()
                .contains("non-PAD")
        );
    }

    #[test]
    fn compact_envelope_carries_canonical_stage_room_cancellation_guards() {
        let mut batch = fixture();
        batch.candidates[0].cancellation_guard = Some(NativeSuffixCancellationGuard {
            allowed_stage_rooms: vec![NativeSuffixStageRoom {
                stage: "F_SP103".into(),
                room: 1,
            }],
        });
        let encoded = encode_compact_suffix_batch(&batch).unwrap();
        assert_eq!(&encoded[..8], b"DSKSBX\x02\0");
        assert!(encoded.windows(7).any(|window| window == b"F_SP103"));

        batch.candidates[0]
            .cancellation_guard
            .as_mut()
            .unwrap()
            .allowed_stage_rooms
            .push(NativeSuffixStageRoom {
                stage: "F_SP103".into(),
                room: 1,
            });
        assert!(encode_compact_suffix_batch(&batch).is_err());
    }

    #[test]
    fn variable_candidate_horizons_and_selected_retention_are_compact() {
        let mut batch = fixture();
        batch.schema = NATIVE_VARIABLE_CACHED_SUFFIX_BATCH_SCHEMA.into();
        let cache = batch.checkpoint_cache.as_mut().unwrap();
        cache.retain_candidate_checkpoints = false;
        cache.retain_candidate_index = Some(1);
        batch.candidates[0].maximum_ticks = Some(2);
        if let MacroAction::PadRun { frames, .. } = &mut batch.candidates[0].actions[0] {
            *frames = 2;
        }
        let mut second = batch.candidates[0].clone();
        second.id = "candidate-2".into();
        second.maximum_ticks = Some(4);
        if let MacroAction::PadRun { frames, .. } = &mut second.actions[0] {
            *frames = 4;
        }
        batch.candidates.push(second);

        let encoded = encode_compact_suffix_batch(&batch).unwrap();
        assert_eq!(&encoded[..8], &COMPACT_SUFFIX_BATCH_MAGIC);
        assert!(encoded.len() * 3 < serde_json::to_vec_pretty(&batch).unwrap().len());

        batch
            .checkpoint_cache
            .as_mut()
            .unwrap()
            .retain_candidate_index = Some(2);
        assert!(encode_compact_suffix_batch(&batch).is_err());
    }
}
