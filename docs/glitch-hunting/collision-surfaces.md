# Cached collision-surface observation

This document fixes the first wire and safety contract for joining Link's
already-resolved collision cache to static world data. It is instrumentation,
not gameplay implementation. It must never issue a collision query, invoke a
gameplay transition, populate a cache, or write through a game pointer.

## Native boundary

The sampler runs post-simulation under the default-off
`DUSK_ENABLE_AUTOMATION_OBSERVERS` gate. It copies the six bounded surfaces
already retained by `dBgS_Acch`, in this canonical order:

1. ground;
2. roof;
3. water;
4. wall circle 0;
5. wall circle 1;
6. wall circle 2.

The reader validates the cached `cBgS_PolyInfo`, the background slot, and
`ChkSafe` before inspecting backing data. It then discriminates the concrete
backing format and reads immutable tables directly:

- DZB: `cBgD_t` triangle, material, group, and source-index records;
- KCL/PLC: `KC_PrismData` plus its `sBgPc` attribute record.

It does **not** call `dBgS::GetExitId`, any `dBgW_Base` material virtual,
`GetTriPla`, `GetTriPnt`, `getPrismData`, `getPolyCode`, `GroundCross`,
`LineCross`, `CrrPos`, or an asserting/lazy helper. KCL access may use only a
guarded forward/friend declaration in `dBgWKCol` and `dBgPlc`; the adapter body
remains out of line in `src/dusk/automation`. No gameplay method, member,
layout, vtable, or control-flow change is permitted.

## Trace v2 channel 10

`PlayerCollisionSurfaces` version 1 is optional and has a fixed 496-byte stride:
a 16-byte set header followed by six 80-byte surface records. A fixed bound
keeps native work and output predictable. The channel is known but is not added
to the default channel set until its cost has been measured. Requesting it also
requires the Stage channel: stage identity is part of the static join, and all
pending-stage flags and masks must be cross-checked against the same record's
Stage payload rather than becoming unverified duplicated state.

The set header is:

| Offset | Type | Meaning |
| ---: | --- | --- |
| 0 | `u32` | set flags |
| 4 | `i8` | Link room used by collision-exit SCLS lookup |
| 5 | `u8` | cached identities present |
| 6 | `u8` | backing code records resolved |
| 7 | `u8` | SCLS destinations resolved |
| 8 | `u16` | raw Link explicit-exit field |
| 10 | `u8` | bit mask, in canonical slot order, of surfaces whose destination equals the same-tick pending stage |
| 11 | 5 bytes | zero |

Set flags are fixed as bit 0 `current-room-valid`, bit 1
`explicit-link-exit-present`, and bit 2 `next-stage-pending`; all other bits are
zero. The explicit-exit bit is important: when it is set, a ground polygon's
exit metadata must not be claimed as the transition source.

Each surface record is:

| Offset | Type | Meaning |
| ---: | --- | --- |
| 0 | `u32` | presence/provenance flags |
| 4 | `u8` | kind: ground, roof, water, or wall |
| 5 | `u8` | wall slot, otherwise zero |
| 6 | `u8` | backing format: none, DZB, or KCL |
| 7 | `u8` | raw-code word presence mask |
| 8 | `u16` | session background slot |
| 10 | `u16` | local polygon/prism index |
| 12 | `u32` | optional owner session process ID |
| 16 | `u16` | material/attribute row |
| 18 | `u16` | DZB group row, otherwise absent |
| 20 | 5 x `u32` | normalized raw collision code words |
| 40 | `u8` | raw exit ID (`0x3f` means no exit; `0xff` means unavailable) |
| 41 | `i8` | exact or inferred source room |
| 42 | `i8` | SCLS source room |
| 43 | `i8` | destination room |
| 44 | `i8` | destination layer |
| 45 | `u8` | destination wipe |
| 46 | `u8` | destination wipe time |
| 47 | `i8` | destination time hour |
| 48 | `i16` | destination point |
| 50 | `u8` | source-geometry index count |
| 51 | `u8` | zero |
| 52 | 6 x `u16` | DZB vertex or KCL position/normal indices |
| 64 | `f32` | KCL prism height when present |
| 68 | 8 bytes | fixed destination stage name |
| 76 | 4 bytes | zero |

Surface flags are fixed as:

| Bit | Meaning |
| ---: | --- |
| 0 | cached session identity present |
| 1 | owner session process ID present |
| 2 | backing collision record safely resolved |
| 3 | at least one raw collision-code word present |
| 4 | material/attribute row present |
| 5 | DZB group row present |
| 6 | source room present |
| 7 | source room is an exact room-background pointer match |
| 8 | SCLS source room present |
| 9 | SCLS destination present |
| 10 | destination equals the same-record pending stage tuple |
| 11 | source-geometry indices present |
| 12 | finite KCL prism height present |

All higher bits are zero. Owner, backing, codes, material, group, source room,
SCLS, destination, geometry, and height presence imply session identity. Bit 7
implies bit 6; bits 8 through 10 are ground-only in version 1; bit 10 implies
bits 8 and 9 plus set flag 2. The raw-code mask uses bits 0 through 4 for the
five words and is zero exactly when surface flag 3 is clear.

The raw words preserve game data instead of baking every material bitfield into
the wire schema. For DZB they are material `info0`, `info1`, `info2`, pass flags,
and group info. For KCL they are PLC `code0` through `code4`. `raw_exit_id` must
equal the low six bits of word zero whenever codes are present.

Source-room provenance distinguishes an exact room-background pointer match
from an owner-room inference. Session background slots and process IDs are
diagnostics, never portable IDs. The portable join key is the run's game-data
identity plus stage, source artifact kind, source room, local polygon index, and
the raw/source-index consistency tuple. An inferred source room stays inferred
after the join and is accepted only if the static inventory finds one exact
consistent candidate.

Only the ground record resolves an SCLS destination in version 1, because that
is the audited Link call path. The SCLS source is Link's current room, not the
collision group, owner, or inferred source room. Roof, water, and wall exit bits
remain raw material facts; they are not described as active transitions.

## Missingness and validation

The channel is `Absent` when there is no player, `Unavailable` for a non-Link
player, and `Present` for Link even when all six records are empty. Each slot has
a fixed kind/slot discriminator. All absent optional values use these canonical
representations and require their presence flag to be clear:

- `u16` background, polygon, material, group, and geometry indices: `0xffff`;
- owner process ID: `0xffffffff`;
- raw exit, wipe, and wipe-time bytes: `0xff`;
- absent signed room, layer, and time-hour fields: `-128` (because `-1` is a
  valid value in several of those domains);
- absent destination point: `-32768`;
- raw code words, destination name, and absent KCL height: zero.

The set's current room is valid only when set flag 0 is present and is then
`-1` or `0..63`; otherwise it is `-128`. `raw_link_exit` is always copied for a
Link sample, including the ordinary `0x003f` no-override value. Destination
rooms are `-1..63`, layers are `-1..14`, points are nonnegative, and time-hour
is `-1..30` when a destination is present.

Native serialization and Rust decoding both reject:

- incoherent flags, counts, masks, sentinels, kinds, slots, or formats;
- owner/code/material/geometry/destination fields without a safe identity;
- raw exit IDs that disagree with raw word zero;
- destination data without a valid ground exit and SCLS source;
- malformed fixed names or destination ranges;
- next-stage match bits that disagree with the same record's Stage channel;
- nonzero reserved bytes or non-finite KCL heights.

When channel 9 and channel 10 are both present, each cached identity and owner
must agree. `movement-state/v1` rejects channel 10 explicitly rather than
silently discarding a state-bearing channel.

## F_SP103 acceptance proof

The checked first-exit tape must show the cached ground polygon changing from
room-1 KCL prism 841 to prism 2217 on the tick where `F_SP104` becomes pending.
Prism 2217 must resolve through PLC attribute 19 to exit ID 1 and the room-1
SCLS destination `F_SP104`, room 1, point 0. The unrelated live `SCENE_EXIT`
actor must remain separately observable as outside its volume and pointing to
`F_SP103`.

Three cold runs must produce byte-identical complete traces. Any divergence is
a framework determinism bug, not a reason to weaken or add slack to the tape.
