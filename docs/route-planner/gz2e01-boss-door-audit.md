# GZ2E01 L1 boss-door family audit

This audit applies only to the exact bundled GameCube USA 1.0 `GZ2E01`
fingerprint. Product ID or actor name alone is not evidence that another build
has identical code, resources, parameter layouts, or event programs.

## Source family

`src/d/d_stage.cpp` maps `L1Bdoor`, `L2Bdoor`, `L4Bdoor`, `L6Bdoor`,
`L7Bdoor`, `L8Bdoor`, and `L9Bdoor` to `fpcNm_L1BOSS_DOOR_e`. `L3Bdoor` and
`L5Bdoor` use different actor families and are intentionally excluded.

The audited source identities are:

| Source | SHA-256 | Relevant behavior |
| --- | --- | --- |
| `src/d/actor/d_a_door_bossL1.cpp` | `221c170e034cf90cc43b20dc737bebeb44d6f8b54111d4454024f2fea7069d79` | Offer guard, interaction test, keyhole/event phases, unlock switch, collision release, and scene change |
| `src/d/d_door_param2.cpp` | `b0dacfc4b9c46786d73a840e55385e535364b9fee7de66cd0e2af18f25d1ca78` | Front/back room, exit, and switch decoding |
| `src/d/d_stage.cpp` | `5c46ffc79e891b59b02455b837d9966d05c147d8d95c91c65cc845dd848d32ad` | Actor-name-to-process mapping |
| `include/d/d_save.h` | `74a211e5d2ee2c0fe4ce259905fe1f479f373d5b2459d654871cbbd2f61e8756` | `dSv_memBit_c` switch and dungeon-item backing layout |

The common parameter decoder reads:

- front room from parameter bits 13–18;
- back room from bits 19–24;
- SCLS exit index from bits 25–30; and
- unlock switch ID from the low byte of home angle Z.

## Hard offer guard

`checkOpen()` does not treat an encoded destination as sufficient. It requires:

1. the boss-key bit in the currently bound dungeon memory;
2. the usable side selected by `checkFront()`; and
3. the actor-local `checkArea()` interaction test.

The planner imports the first two decidable conditions as hard semantics. The
boss-key read is `dSv_memBit_c` byte `0x1d`, mask `0x04`, resolved through the
current-stage backing. `checkFront()` rejects room 50 for ordinary stages and
room 10 specifically in `D_MN08A`; reverse-side placements do not become
candidate transitions.

The remaining physical test is an explicit unresolved interaction obligation.
In actor-local coordinates it requires `|x| <= 200`, `|z| <= 100`, and a facing
delta no greater than `0x4000`. Wolf form additionally uses attention position
and requires current-position `|x| <= 130`. The importer records these values
without pretending that unoriented placement points prove them.

## Unlock and scene-change phases

The source keeps these operations distinct:

1. resources and the door actor load;
2. the event is offered;
3. `INIT` selects whether this is the first unlock;
4. `UNLOCK` drives the keyhole actor when present;
5. action 17 writes the persistent switch;
6. `OPEN` releases collision and advances animation;
7. `CHG_SCENE` calls `onSceneChangeArea(exit_no, ...)`; and
8. restart/close handling completes around the boundary.

The current world importer exposes a collapsed transition only as an upper-level
candidate. It retains an unknown actor-state obligation covering resource load,
keyhole, event, collision, scene-change, and restart phases. Thus the solver
cannot infer traversal merely from the hard flag guard. A later phase-program
import can replace that obligation without changing the boss-key or switch
backing semantics.

For memory-switch IDs below `0x80`, the successful candidate performs a masked
`write_bound_raw` against current-stage dungeon memory before setting the SCLS
destination. `dSv_memBit_c::mSwitch` begins at byte `0x08` and consists of four
big-endian 32-bit words. Other switch domains remain unimported rather than being
written to the wrong backing store.

## Forest Temple representative placement

The audited `D_MN05/R12_00.arc` placement is:

- archive SHA-256 `5b495a915c1539b92f57e84f7cbcf0b5662a8caeaf7ecf0503ac15af7a6e6a77`;
- `room.dzr` SHA-256 `9336aabaee513b635d6d0d3db3f5f3b67f5c6bd6643581ebd1a8f7b779fa8e7a`;
- actor `L1Bdoor`, parameters `0x01918000`;
- position `(7283, 3302, -16430)`, angles `(-211, 0, 0x1717)`;
- decoded front room 12, back room 50, exit index 0, switch `0x17`; and
- exit 0 to `D_MN05A`, room 50, spawn 0.

Switch `0x17` maps to raw byte `0x09`, mask `0x80`. The reciprocal boss-room
placement is not imported as a traversable reverse edge because room 50 fails
`checkFront()`.

## Conservative import boundary

The importer emits a boss-door candidate only when all of the following hold:

- the complete content fingerprint is the audited GZ2E01 identity;
- the placement name maps to the audited L1 boss-door actor family;
- the placement is on a source-audited usable side;
- exactly one same-room SCLS record matches the decoded exit index; and
- the switch belongs to the modeled memory-switch domain.

Missing or ambiguous exits, different builds, reverse-side actors, and unmodeled
switch domains remain encoded facts with no invented candidate. Tests cover the
Forest Temple guard, switch write, destination binding, exact-build restriction,
reverse-side rejection, and atomic backing-sensitive raw writes.
