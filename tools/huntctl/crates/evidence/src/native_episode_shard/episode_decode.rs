//! Decode episodes and validate their pre/action/post boundary joins.

use super::*;

pub(super) fn decode_episode(
    reader: &mut Reader<'_>,
    maximum_ticks: u32,
    source_frame: u64,
    observation_version: u16,
) -> Result<(NativeEpisode, u64, u64), NativeEpisodeShardError> {
    if reader.bytes(4)? != EPISODE_MAGIC || usize::from(reader.u16()?) != BLOCK_HEADER_SIZE {
        return Err(NativeEpisodeShardError::new("invalid episode block header"));
    }
    let flags = reader.u16()?;
    if flags & !SUCCESS != 0 {
        return Err(NativeEpisodeShardError::new("unknown episode block flags"));
    }
    let ticks_executed = reader.u32()?;
    let first_hit = reader.u32()?;
    let remaining_ticks = reader.u32()?;
    let id_length = usize::from(reader.u16()?);
    if reader.u16()? != 0 {
        return Err(NativeEpisodeShardError::new(
            "nonzero episode block reserved field",
        ));
    }
    let expanded_size = reader.usize_u64()?;
    let compressed_size = reader.usize_u64()?;
    let payload_xxh3_128: [u8; 16] = reader.bytes(16)?.try_into().expect("exact length");
    if reader.u64()? != 0
        || ticks_executed == 0
        || ticks_executed > maximum_ticks
        || remaining_ticks != maximum_ticks - ticks_executed
        || id_length == 0
        || expanded_size > MAX_EXPANDED_BYTES
    {
        return Err(NativeEpisodeShardError::new(
            "invalid episode block descriptor",
        ));
    }
    let id = std::str::from_utf8(reader.bytes(id_length)?)
        .map_err(|_| NativeEpisodeShardError::new("episode id is not UTF-8"))?
        .to_owned();
    let compressed = reader.bytes(compressed_size)?;
    let expanded = zstd::bulk::decompress(compressed, expanded_size)
        .map_err(|error| NativeEpisodeShardError::new(error.to_string()))?;
    if xxhash_rust::xxh3::xxh3_128(&expanded).to_be_bytes() != payload_xxh3_128 {
        return Err(NativeEpisodeShardError::new(
            "episode payload digest mismatch",
        ));
    }
    let mut payload = Reader::new(&expanded);
    if payload.bytes(8)? != PAYLOAD_MAGIC
        || payload.u16()? != observation_version
        || usize::from(payload.u16()?) != PAYLOAD_HEADER_SIZE
        || payload.u32()? != ticks_executed
        || payload.u32()? != 0
        || payload.u32()? != 0
    {
        return Err(NativeEpisodeShardError::new(
            "invalid expanded episode header",
        ));
    }
    let mut steps = Vec::with_capacity(ticks_executed as usize);
    let success = flags & SUCCESS != 0;
    for step_index in 0..ticks_executed {
        let pre_input = decode_observation(&mut payload, observation_version)?;
        let chosen_pad = decode_pad(&mut payload)?;
        let consumed_pad = decode_pad(&mut payload)?;
        let post_simulation = decode_observation(&mut payload, observation_version)?;
        validate_step(
            steps.last(),
            &pre_input,
            consumed_pad,
            &post_simulation,
            step_index + 1 == ticks_executed,
            success,
        )?;
        steps.push(NativeEpisodeStep {
            pre_input,
            chosen_pad,
            consumed_pad,
            post_simulation,
        });
    }
    if !payload.done() {
        return Err(NativeEpisodeShardError::new(
            "trailing expanded episode bytes",
        ));
    }
    let first_hit_tick = (first_hit != u32::MAX).then_some(first_hit);
    if success != first_hit_tick.is_some()
        || first_hit_tick.is_some_and(|tick| tick + 1 != ticks_executed)
        || steps.first().is_none_or(|step| {
            step.pre_input.remaining_ticks != maximum_ticks
                || step.pre_input.tape_frame != source_frame
        })
        || steps
            .last()
            .is_none_or(|step| step.post_simulation.remaining_ticks != remaining_ticks)
        || steps
            .last()
            .is_none_or(|step| step.post_simulation.goal.reached != success)
    {
        return Err(NativeEpisodeShardError::new(
            "episode outcome disagrees with terminal boundary",
        ));
    }
    Ok((
        NativeEpisode {
            id,
            success,
            ticks_executed,
            first_hit_tick,
            remaining_ticks,
            payload_xxh3_128,
            steps,
        },
        expanded_size as u64,
        compressed_size as u64,
    ))
}

pub(super) fn validate_step(
    prior: Option<&NativeEpisodeStep>,
    pre: &NativeLearningObservation,
    action: NativeRawPad,
    post: &NativeLearningObservation,
    final_step: bool,
    success: bool,
) -> Result<(), NativeEpisodeShardError> {
    if pre.phase != NativeObservationPhase::PreInput
        || post.phase != NativeObservationPhase::PostSimulation
        || pre.simulation_tick != post.simulation_tick
        || pre.tape_frame != post.tape_frame
        || post.boundary_index != pre.boundary_index + 1
        || post.remaining_ticks + 1 != pre.remaining_ticks
        || post.previous_input != action
        || pre.terminal_reason != NativeTerminalReason::None
        || (!final_step && post.terminal_reason != NativeTerminalReason::None)
        || (final_step
            && post.terminal_reason
                != if success {
                    NativeTerminalReason::GoalReached
                } else {
                    NativeTerminalReason::TickBudgetExhausted
                })
    {
        return Err(NativeEpisodeShardError::new(
            "action is not aligned to its observation boundaries",
        ));
    }
    if let Some(prior) = prior {
        let mut mismatches = Vec::new();
        if prior.post_simulation.state_identity != pre.state_identity {
            mismatches.push(format!(
                "state_identity {:02x?} != {:02x?}",
                prior.post_simulation.state_identity, pre.state_identity
            ));
        }
        if prior.post_simulation.boundary_index != pre.boundary_index {
            mismatches.push(format!(
                "boundary_index {} != {}",
                prior.post_simulation.boundary_index, pre.boundary_index
            ));
        }
        if prior.post_simulation.remaining_ticks != pre.remaining_ticks {
            mismatches.push(format!(
                "remaining_ticks {} != {}",
                prior.post_simulation.remaining_ticks, pre.remaining_ticks
            ));
        }
        if prior.post_simulation.simulation_tick + 1 != pre.simulation_tick {
            mismatches.push(format!(
                "simulation_tick {} + 1 != {}",
                prior.post_simulation.simulation_tick, pre.simulation_tick
            ));
        }
        if prior.post_simulation.tape_frame + 1 != pre.tape_frame {
            mismatches.push(format!(
                "tape_frame {} + 1 != {}",
                prior.post_simulation.tape_frame, pre.tape_frame
            ));
        }
        if prior.consumed_pad != pre.previous_input {
            mismatches.push("consumed_pad != previous_input".to_owned());
        }
        if !mismatches.is_empty() {
            return Err(NativeEpisodeShardError::new(format!(
                "adjacent transition boundaries are discontinuous: {}",
                mismatches.join(", ")
            )));
        }
    }
    Ok(())
}
