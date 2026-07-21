# GZ2E01 L5 boss-door audit

This audit applies only to the exact bundled GameCube USA 1.0 `GZ2E01`
fingerprint. It does not generalize the actor or resource layouts to another
revision, platform, or language bundle.

## Source and parameter family

`src/d/d_stage.cpp` maps only `L5Bdoor` to `fpcNm_L5BOSS_DOOR_e`. The actor is
therefore kept separate from the L1-family importer even though both use the
common `dDoor_info_c` parameter decoder.

| Source | SHA-256 | Relevant behavior |
| --- | --- | --- |
| `src/d/actor/d_a_door_bossL5.cpp` | `9f649b99f027e39f1d39ce066d815a78032b536c4a9a83e0361681af2265102e` | Human-form, boss-key, usable-side, interaction, keyhole, event, collision, scene-change, and restart behavior |
| `src/d/d_door_param2.cpp` | `b0dacfc4b9c46786d73a840e55385e535364b9fee7de66cd0e2af18f25d1ca78` | Front/back room, exit, and switch decoding |
| `src/d/d_stage.cpp` | `5c46ffc79e891b59b02455b837d9966d05c147d8d95c91c65cc845dd848d32ad` | Actor-name-to-process mapping |
| `include/d/d_save.h` | `74a211e5d2ee2c0fe4ce259905fe1f479f373d5b2459d654871cbbd2f61e8756` | Boss-key and memory-switch backing layout |

The common decoder reads the front room from parameter bits 13–18, back room
from bits 19–24, SCLS exit index from bits 25–30, and unlock switch ID from the
low byte of home angle Z.

## Hard guards and physical obligation

`checkOpen()` rejects wolf form before checking the current dungeon's boss-key
bit. The imported hard guard consequently requires:

1. the source location;
2. player form `human`; and
3. current-stage dungeon-memory byte `0x1d`, mask `0x04`.

The offer also depends on actor-local geometry. `checkFront()` requires local Z
to be positive. `checkArea()` requires `|x| <= 200`, `|z| <= 100`, and a facing
delta no greater than `0x4000`. Until extracted oriented geometry can prove
those facts, they remain an explicit interaction obligation rather than a
synthetic Boolean or an assumed traversable edge.

## Two retail placements

The exact GZ2E01 extraction contains two `L5Bdoor` records.

### Dungeon side

`D_MN11/R04_00.arc` contains:

- archive SHA-256 `6ee1274731222f3abe62c50de686fbae60663ba11e911f79c54fe886e221cd55`;
- `room.dzr` SHA-256 `cd32b1ac737b8cfe6f92fa35a18ba4e24a5ccbfe8b466ae403b6c269dcbfc5c3`;
- parameters `0x05908200`, position `(0, 2109, -5237)`, and angles
  `(-1, 0, -248)`;
- decoded front room 4, back room 50, exit index 2, and switch `0x08`; and
- exit 2 to `D_MN11A`, room 50, spawn 1.

`D_MN11` has stage type 1. When the switch is initially clear, the actor creates
the keyhole. The successful unlock path opens it and writes switch `0x08` before
the scene change. In `dSv_memBit_c::mSwitch`, that is a masked write to byte
`0x0a`, mask `0x01`, through the current-stage dungeon-memory binding.

### Boss-room side

`D_MN11A/R50_00.arc` contains:

- archive SHA-256 `4acd3b8ce5ac24820364314c1cbec9569bf0faad2d4f0e6688e974616d8c7889`;
- `room.dzr` SHA-256 `106533086f77371b6abd4cfea2d0d2c14fd88f5ce1a2569bfc8020848d2519a6`;
- parameters `0x03908200`, position `(0, 0, 2100)`, and angles
  `(-1, 0, -248)`;
- decoded front room 4, back room 50, exit index 1, and switch `0x08`; and
- exit 1 to `F_SP114`, room 1, layer 2, spawn 11.

`D_MN11A` has stage type 3. That suppresses keyhole creation. `unlockInit()`
returns when no keyhole exists, so this placement must not inherit the dungeon
side's unlock-switch write. Its candidate effect is the decoded scene change
only.

## Phase boundary and conservative import

The source separates offer, unlock, optional key deletion, collision release,
scene change, close/end, and restart-room writes. The importer keeps those
loaded-actor/event phases as an unresolved actor-state obligation. In
particular, importing the final SCLS destination does not assert that every
intermediate event or restart write is understood.

Candidates are emitted only for an exact GZ2E01 fingerprint, an extracted
`L5Bdoor` scaled-actor placement in `D_MN11` or `D_MN11A`, one unique matching
same-room SCLS record, and a switch in the modeled memory-switch domain.
Unknown builds, stages, ambiguous exits, unsupported switch domains, and
unproven interaction geometry fail closed.

The L1 and L5 fixtures also test the actual extracted placement kind: retail
`Door` chunk records are scaled actors. Accepting only ordinary `ACTR` records
would leave valid retail placements encoded but silently unimported.
