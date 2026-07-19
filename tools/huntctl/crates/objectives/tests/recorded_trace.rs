use dusklight_objectives::milestone_dsl::{evaluate_recorded_trace, parse};
use dusklight_trace::trace::{self, TraceChannel};

const MAGIC: &[u8; 8] = b"DUSKTRCE";
const HEADER_SIZE: usize = 64;
const DIRECTORY_ENTRY_SIZE: usize = 64;
const FILE_COMPLETE: u32 = 1;
const CHANNEL_REQUIRED: u32 = 1;
const CHANNEL_DENSE: u32 = 2;
const CORE_SIMULATION_TICK_VALID: u32 = 1;
const PLAYER_IS_LINK: u32 = 1;

#[test]
fn authored_predicates_evaluate_against_a_decoded_recorded_trace_fixture() {
    let bytes = build_trace(vec![
        (TraceChannel::Stage, stage_payload()),
        (TraceChannel::PlayerMotion, player_payload()),
        (TraceChannel::Event, event_payload()),
        (TraceChannel::Rng, rng_payload()),
    ]);
    let trace = trace::decode(&bytes).unwrap();
    let program = parse(
        r#"milestones 1.3

milestone recorded_hit {
  phase post_sim
  when stage.name == "F_SP103" && stage.room == 1 &&
       player.exists && player.is_link && player.procedure == 7 &&
       player.position.x between 665.0 and 667.0 &&
       player.in_aabb(600.0, 700.0, -2500.0, 700.0, 900.0, -2400.0) &&
       !event.running && event.id == -1 && rng.primary.calls == 100
}

milestone unavailable_actor_catalog_cannot_guess {
  phase post_sim
  when actor.placed.exists("F_SP103", 1, 7, 42)
}
"#,
    )
    .unwrap();

    let hits = evaluate_recorded_trace(&program, &trace).unwrap();
    let hit = hits["recorded_hit"].as_ref().unwrap();
    assert_eq!(hit.record_index, 0);
    assert_eq!(hit.boundary_index, 1);
    assert!(hits["unavailable_actor_catalog_cannot_guess"].is_none());
}

fn build_trace(extra_channels: Vec<(TraceChannel, Vec<u8>)>) -> Vec<u8> {
    let mut core = vec![0; 32];
    core[0..8].copy_from_slice(&1_u64.to_le_bytes());
    core[8..16].copy_from_slice(&0_u64.to_le_bytes());
    core[16..24].copy_from_slice(&u64::MAX.to_le_bytes());
    core[24..28].copy_from_slice(&CORE_SIMULATION_TICK_VALID.to_le_bytes());
    core[28] = 2;
    core[29] = 1;

    let mut channels = vec![(TraceChannel::Core, core)];
    channels.extend(extra_channels);
    let requested = channels
        .iter()
        .fold(0_u64, |mask, (channel, _)| mask | channel.bit());
    let mut bytes = header(channels.len() as u16, requested);
    bytes[20..28].copy_from_slice(&1_u64.to_le_bytes());
    bytes.resize(HEADER_SIZE + channels.len() * DIRECTORY_ENTRY_SIZE, 0);

    for (index, (channel, payload)) in channels.into_iter().enumerate() {
        let descriptor = HEADER_SIZE + index * DIRECTORY_ENTRY_SIZE;
        let status_offset = bytes.len();
        bytes.push(1); // TraceChannelStatus::Present on the v2 wire.
        let payload_offset = bytes.len();
        bytes.extend_from_slice(&payload);

        bytes[descriptor..descriptor + 2].copy_from_slice(&(channel as u16).to_le_bytes());
        bytes[descriptor + 2..descriptor + 4].copy_from_slice(&1_u16.to_le_bytes());
        let flags = CHANNEL_DENSE
            | if channel == TraceChannel::Core {
                CHANNEL_REQUIRED
            } else {
                0
            };
        bytes[descriptor + 4..descriptor + 8].copy_from_slice(&flags.to_le_bytes());
        bytes[descriptor + 8..descriptor + 12]
            .copy_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes[descriptor + 12..descriptor + 16].copy_from_slice(&1_u32.to_le_bytes());
        bytes[descriptor + 16..descriptor + 24]
            .copy_from_slice(&(status_offset as u64).to_le_bytes());
        bytes[descriptor + 24..descriptor + 32].copy_from_slice(&1_u64.to_le_bytes());
        bytes[descriptor + 32..descriptor + 40]
            .copy_from_slice(&(payload_offset as u64).to_le_bytes());
        bytes[descriptor + 40..descriptor + 48]
            .copy_from_slice(&(payload.len() as u64).to_le_bytes());
    }
    bytes
}

fn header(channel_count: u16, requested: u64) -> Vec<u8> {
    let mut bytes = vec![0; HEADER_SIZE];
    bytes[..8].copy_from_slice(MAGIC);
    bytes[8..10].copy_from_slice(&2_u16.to_le_bytes());
    bytes[10..12].copy_from_slice(&(HEADER_SIZE as u16).to_le_bytes());
    bytes[12..16].copy_from_slice(&30_u32.to_le_bytes());
    bytes[16..20].copy_from_slice(&1_u32.to_le_bytes());
    bytes[28..32].copy_from_slice(&FILE_COMPLETE.to_le_bytes());
    bytes[32..34].copy_from_slice(&channel_count.to_le_bytes());
    bytes[34..36].copy_from_slice(&(DIRECTORY_ENTRY_SIZE as u16).to_le_bytes());
    bytes[36..44].copy_from_slice(&(HEADER_SIZE as u64).to_le_bytes());
    let data_offset = HEADER_SIZE + usize::from(channel_count) * DIRECTORY_ENTRY_SIZE;
    bytes[44..52].copy_from_slice(&(data_offset as u64).to_le_bytes());
    bytes[52..60].copy_from_slice(&requested.to_le_bytes());
    bytes
}

fn stage_payload() -> Vec<u8> {
    let mut payload = vec![0; 32];
    payload[0..7].copy_from_slice(b"F_SP103");
    payload[8] = 1;
    payload[9] = u8::MAX;
    payload[10..12].copy_from_slice(&1_i16.to_le_bytes());
    payload[20] = u8::MAX;
    payload[21] = u8::MAX;
    payload[22..24].copy_from_slice(&(-1_i16).to_le_bytes());
    payload
}

fn player_payload() -> Vec<u8> {
    let mut payload = vec![0; 52];
    payload[0..4].copy_from_slice(&19_u32.to_le_bytes());
    payload[4..6].copy_from_slice(&253_i16.to_le_bytes());
    payload[6..8].copy_from_slice(&7_u16.to_le_bytes());
    write_f32(&mut payload, 20, 666.0);
    write_f32(&mut payload, 24, 800.0);
    write_f32(&mut payload, 28, -2431.0);
    write_f32(&mut payload, 32, 1.0);
    write_f32(&mut payload, 44, 3.0);
    payload[48..52].copy_from_slice(&PLAYER_IS_LINK.to_le_bytes());
    payload
}

fn event_payload() -> Vec<u8> {
    let mut payload = vec![0; 16];
    payload[4..6].copy_from_slice(&(-1_i16).to_le_bytes());
    payload[8] = u8::MAX;
    payload
}

fn rng_payload() -> Vec<u8> {
    let mut payload = vec![0; 64];
    payload[0..4].copy_from_slice(&1_u32.to_le_bytes());
    payload[4..8].copy_from_slice(&2_u32.to_le_bytes());
    for (offset, id, states, calls) in [
        (8, 0_u8, [11_i32, 12, 13], 100_u64),
        (36, 1_u8, [21_i32, 22, 23], 200_u64),
    ] {
        payload[offset] = id;
        payload[offset + 4..offset + 8].copy_from_slice(&1_u32.to_le_bytes());
        for (index, state) in states.into_iter().enumerate() {
            payload[offset + 8 + index * 4..offset + 12 + index * 4]
                .copy_from_slice(&state.to_le_bytes());
        }
        payload[offset + 20..offset + 28].copy_from_slice(&calls.to_le_bytes());
    }
    payload
}

fn write_f32(bytes: &mut [u8], offset: usize, value: f32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_bits().to_le_bytes());
}
