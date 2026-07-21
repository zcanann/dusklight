# GZ2E01 Forest Temple small-key door audit

This audit follows one concrete keyed room transition from extracted placement
data through the live actor and save backing. It is intentionally narrower than
an audit of every keyed-door family.

## Placement and encoded adjacency

The representative placement is `Door[1]` in `stage.dzs` from
`orig/GZ2E01/files/res/Stage/D_MN05/STG_00.arc` (SHA-256
`9d08ac55fce27a6a741a6a502a4a2502146c3ff91abeb7d8c44824a6df8325a4`).

| Field | Raw/decoded value |
| --- | --- |
| Actor | `yodoor` / `DOOR20` |
| Raw record | `796f646f6f7200006c10220146231000456a800045983000bdffc000ff0b00ff0a0a0aff` |
| Parameters | `0x6c102201` |
| Position | `(10436.0, 3752.0, 4870.0)` |
| Angles | `(-16897, -16384, -245)`; Z bytes `0xff0b` |
| Door kind | `1`, wooden rolling shutter |
| Front option | `2`, small-key lock |
| Back option | `0` |
| Front room | `1` |
| Back room | `2` |
| Unlock switch | `0x0b` |
| Secondary switch | `0xff`, absent |

The parameter readers in `src/d/d_door_param2.cpp` establish the bit layout:
kind `0..4`, front option `8..9`, back option `10..12`, front room `13..18`,
back room `19..24`, and the primary switch in the low byte of angle Z.

This encodes a room adjacency from `D_MN05/R01` to `D_MN05/R02`; it does **not**
encode permission to traverse it. Unlike an `SCLS` exit, the shutter derives its
next room from the side currently occupied. `openInitCom()` records the opposite
room as the next stay room. A route-planner importer may therefore surface the
candidate edge immediately, but must leave its activation contract unsatisfied.

## Guard and one-time key consumption

`daDoor20_c::setEventPrm()` in
`src/d/actor/d_a_door_shutter.cpp` offers the door event only when all relevant
runtime conditions hold, including:

- the adjoining room is loaded;
- Link's form and Midna state permit this door interaction;
- door-stop and spatial/facing checks pass; and
- when the live actor is locked, the active dungeon key count is nonzero.

The acceptance fixture represents the adjoining-room and approach checks as
live actor fields. Geometry remains an activation prerequisite instead of being
inferred from the encoded room numbers.

On event demo action 8, the actor first checks that switch `0x0b` is not already
set and that either side has option `2`. It then writes switch `0x0b` and queues
a `-1` key delta. The HUD update path `dMeter2_c::moveKey()` applies that pending
delta to `dSv_memory_c::mBit.mKeyNum` and clears the delta. This distinction
matters for exact intermediate snapshots: the persistent unlock can be written
one update before the saved key count changes. The switch guard makes repeated
action-8 evaluation idempotent for both writes.

Both values belong to the active stage save bank. `dSv_info_c::onSwitch()` sends
switches below `MEMORY_SWITCH` to `mMemory.getBit()`, while
`dComIfGs_getKeyNum()` reads `mMemory.getBit().getKeyNum()`. Keys from different
dungeon bindings therefore cannot satisfy this door without an explicit backing
rebind/transfer rule.

## Live actor state and reconstruction

The persistent bank does not itself remove collision:

1. `CreateInit()` registers the door background collision.
2. `setKey()` reconstructs `field_0x5f0` as locked exactly when this is a keyed
   door and switch `0x0b` is clear.
3. `createKey()` creates the keyhole child only for that locked state.
4. Demo action 8 waits for the keyhole animation, then clears the live locked
   field.
5. `openInit()` releases background collision and starts the opening animation.
6. `openProc()`/`openEnd()` finish the opening action.
7. `closeInit()` re-registers collision; `closeEnd()` deletes the key child and
   finishes the room crossing.

Consequently, `switch_0b == true` means “unlocked across reloads,” not “currently
open” and not “collision absent.” Reloading reconstructs an unlocked, closed,
collision-registered door with no keyhole. Crossing still requires another live
open sequence. The source-backed solver fixture keeps persistent unlock, pending
key delta, actor lock/keyhole, animation, collision, and current room as separate
state fields and transitions.

## Proven and unproven scope

This proves the full chain for the GZ2E01 `DOOR20` placement above. It does not
claim that boss doors, knob doors, barred gates, or other keyed actor families
share the same event program. Those remain separate audit work. Other builds
also require placement/resource comparison before this record can be treated as
content-equivalent.
