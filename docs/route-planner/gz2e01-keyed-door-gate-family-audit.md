# GZ2E01 keyed door and gate family audit

This audit applies only to the exact bundled GameCube USA 1.0 `GZ2E01`
fingerprint. It completes the source and retail-placement census for gameplay
actors that read the current dungeon's small-key count as part of a door or gate
interaction. It does not claim layout or resource equivalence for another
revision, platform, or language bundle.

The earlier `DOOR20`, L1-family boss-door, and L5 boss-door audits remain the
authoritative family-specific records for those actors. This document covers
the remaining exact-build census and records which semantics are executable in
`extracted-world-facts/v10` and which remain explicit boundaries.

## Census method and source identities

The source census searched every exact call to `dComIfGs_getKeyNum()` and every
queued decrement through `dComIfGp_setItemKeyNumCount(-1)`. UI, debug, and
observer-only reads were separated from gameplay consumers. The resulting
door/gate families are `DOOR20`, mini-boss doors, rider gates, Koki gates,
caravan gates, and key shutters.

| Source | SHA-256 | Audited role |
| --- | --- | --- |
| `src/d/actor/d_a_door_mbossL1.cpp` | `94b00ab791e96a5738a0c2ef94945461c4e930b6128fc5a16d13630da9d1dff2` | Mini-boss front/back options, key guard, switch/decrement, event, collision, and scene change |
| `src/d/actor/d_a_obj_kshutter.cpp` | `dca04961403031ef232059f5f9f8997d2f0a3965b111e97d9d72604e0014d14b` | Key-shutter type/check-key decoder, small-key or boss-key guard, switch, decrement, event, and reconstruction |
| `src/d/actor/d_a_obj_kgate.cpp` | `55696f32a444f9fde4b446442211cc3bed8b2872c8b05d7646001bd3659879e8` | Koki-gate variants, offer geometry, decrement, switch, and post-unlock pushing |
| `src/d/actor/d_a_obj_rgate.cpp` | `eb644962c9c9596514d552e2f87015f1c68786bf998ff79d41a606276750bffb` | Rider-gate key path, switch path, event-bit bypass, and pushing |
| `src/d/actor/d_a_obj_crvgate.cpp` | `f0916a79d3b157454dd2263307567e472d4f394d61ad8ece9153500d91943697` | Paired caravan-gate key path and destructive boar/event path |
| `src/d/actor/d_a_L7demo_dr.cpp` | `7b350f2e3efa4ddb5907b38d4f1f8ceb91d37cc741dce7e4d7de67d436421b02` | Key-gated bridge demo that does not consume a key |
| `src/d/actor/d_a_obj_smallkey.cpp` | `d77d584b1fa5908098ec073a90ed391e0e51c632eb689e30e8f1f65c842c6cf6` | Exceptional F_SP118 collected-chest cleanup that clears the current key count; not a door |
| `src/d/actor/d_a_door_boss.cpp` | `2ebdabd62350147da77580b0992b51e20dac7d470a4bac96fab7b4d1677bf800` | Generic `bdoor` boss-key family; no exact retail placement |
| `src/d/d_meter2.cpp` | `b58ed135700865df0f0cb9ce0e4115de6ec1f9f6dbb8fff8cc1ff99b437d5569` | Queued key-delta commit and clamp |
| `src/d/d_door_param2.cpp` | `b0dacfc4b9c46786d73a840e55385e535364b9fee7de66cd0e2af18f25d1ca78` | Mini-boss front/back room, option, exit, and switch decoding |
| `src/d/d_stage.cpp` | `5c46ffc79e891b59b02455b837d9966d05c147d8d95c91c65cc845dd848d32ad` | Exact actor-name-to-process mappings |
| `include/d/d_save.h` | `74a211e5d2ee2c0fe4ce259905fe1f479f373d5b2459d654871cbbd2f61e8756` | Current dungeon memory backing layout |

The true door/gate decrement sites are `DOOR20`, mini-boss doors, rider gates,
Koki gates, caravan gates, and key shutters. The extra key-shutter decrement
sites occur in older family-local demo procedures; retail event selection uses
the audited procedures below. `d_a_L7demo_dr.cpp` is a guard-only consumer, and
the small-key actor's F_SP118 cleanup is not an unlock.

## Key, boss-key, and switch backing

All of these actors read the currently bound `dSv_memBit_c`. Its memory-switch
array begins at byte `0x08`, the small-key count is byte `0x1c`, and the
dungeon-item byte is `0x1d`; the boss-key mask is `0x04`.

`dComIfGp_setItemKeyNumCount(-1)` does not immediately edit byte `0x1c`. It
queues a HUD/gameplay delta. `dMeter2_c::moveKey()` later adds the delta to the
current count, clamps the result to `[0, 99]`, writes the stage-memory byte, and
clears the pending delta. This timing matters for the Lakebed boss shutter: its
hard guard checks only the boss key, yet its `UNLOCK` cut still queues a small-
key decrement. Counts 1 through 100 lose one, zero clamps back to zero, and a
raw count above 100 clamps to 99.

The v10 candidate effect is the ordinary eventual committed-count equivalent,
not a claim that the transient queue and raw byte change simultaneously. Its
actor-state obligation includes an uncontended pending delta. V9 splits zero,
normal-range, and high-raw-count outcomes so the final committed byte remains
exact; overlapping pickup/unlock deltas still require a later transient queue
model rather than an invented ordering.

Memory-switch IDs below `0x80` map to the four 32-bit words at `0x08..0x17`.
IDs `0x80` and above are not imported as memory switches. An absent `0xff`
switch is also not converted into a persistent write.

## Exact retail placement census

Strict `huntctl world inventory` extraction covered 78 of the 79 stage
directories. `D_MN09` failed closed on an invalid or unordered collision prism.
The planner's placement-only `extract-stage-data` decoder was then run on all 14
`D_MN09` stage archives. It found none of the exact keyed families below, so the
actor census is complete without weakening collision validation. The generic
`bdoor` actor has zero placements in all 79 directories.

### Front-side keyed mini-boss doors

The exact extraction contains six placements whose current room is the decoded
front room, whose front option is `2`, whose switch is a memory switch, and
whose decoded exit resolves uniquely:

| Stage/room | Actor | Parameters | Switch | Exit | Destination |
| --- | --- | --- | --- | --- | --- |
| `D_MN06/7` | `L6Mdoor` | `0x01b0e600` | `0x1d` | 0 | `D_MN06B`, room 51, spawn 0 |
| `D_MN07/3` | `L7door` | `0x03b06200` | `0x06` | 1 | `D_MN07`, room 4, spawn 1 |
| `D_MN08/2` | `L8Mdoor` | `0x01b04600` | `0x06` | 0 | `D_MN08B`, room 51, spawn 0 |
| `D_MN08/5` | `L8Mdoor` | `0x01b0a600` | `0x07` | 0 | `D_MN08C`, room 52, spawn 0 |
| `D_MN08/7` | `L8Mdoor` | `0x03b0e600` | `0x25` | 1 | `D_MN08`, room 11, spawn 0 |
| `D_MN08/11` | `L8Mdoor` | `0x01b16600` | `0x33` | 0 | `D_MN08`, room 8, spawn 0 |

The source implements only a front-side key option: `checkBackKey()` always
returns false. Front option `0` is a switch gate; options `1` and `3` are
stopper gates. Rooms 51 and 52 are forced non-front sides. An ordinary accepted
key opening writes the decoded switch and queues `-1`; a later opening with the
switch already set changes scene without another decrement. `STAFF_SHUTTER` is
an alternate demo activation path and may write the separate third switch, so
it remains an actor/event obligation rather than an invented key branch.

### Key shutters

The low parameter byte is the switch. Bits 8–15 are an authored type that is
incremented with wrapping arithmetic at runtime, bits 16–23 select an event,
and bit 31 enables the actor's internal key check.

| Stage/room | Actor | Parameters | Runtime type | Exact behavior |
| --- | --- | --- | --- | --- |
| `D_MN01/3` | `L3Bdoor` | `0x80ff0255` | 3, Lakebed boss | Boss-key guard; switch `0x55`; `UNLOCK` also queues the incidental small-key decrement |
| `D_MN01/9` | `kshtr00` | `0x80ff0123` | 2 | Small-key guard; switch `0x23`; one-key decrement |
| `D_MN01/10` | `kshtr00` | `0x80ff0124` | 2 | Small-key guard; switch `0x24`; one-key decrement |
| `D_MN05/5` | `kshtr00` | `0x80ffff2b` | 0 by wrapping | Small-key guard; switch `0x2b`; one-key decrement |
| `D_MN05/7` | `kshtr00` | `0x80ffff3e` | 0 by wrapping | Small-key guard; switch `0x3e`; one-key decrement |
| `R_SP116/6` | `vshuter` | `0x00ff03ef` | 4 | `checkKey == 0`; waits for external switch `0xef`, with no internal key guard or writer |

For internally checked shutters, accepting the event writes the switch before
the later `UNLOCK` cut queues the decrement. A set switch reconstructs the actor
open with background collision released. The `vshuter` placement is therefore
not a keyed unlock transition.

### Koki, rider, and caravan gates

All eight exact `K_Gate` placements use name argument/type 0. F_SP108 room 3
has locked switch `0x0c` copies on layers 0 and 14, plus already-unlocked
`0xff` copies on layers 1, 2, 3, 5, and 12. Room 5 has a locked base-layer
switch `0x14` placement. Type 0 requires a key, facing, local X in
`[-100, 100]`, and local Z in `[0, 100]`; accepted activation queues `-1` and
writes the switch. On the live instance, the switch-set branch permits
player/horse/coach contact to drive the two leaves. A type-0 instance created
with its switch already set, or with switch `0xff`, instead initializes at the
fully open plus/minus 80-degree angles and disables further gate action. The
source also has type 1 bomb deletion and type 2 rider behavior, but this census
contains no exact retail placement of either.

`R_Gate` has 14 layered copies in F_SP109 room 0 using switch `0x6b`, plus
F_SP121 room 3 switch `0x82` and room 15 switch `0x81`. Its ordinary keyed path
uses the same key/facing/local-box test, queues `-1`, and writes the switch.
However, persistent event-bit label 68 suppresses the key event and forces the
gate fully open without a key or switch. The F_SP121 switches also belong to an
unmodeled non-memory domain. No executable transition is imported until that
event-bit backing and bypass are represented together.

`CrvGate` has paired parent/child placements in F_SP118 rooms 1 and 2, both with
parameters `0xffffffff`. Its accepted key event requires a key, facing, and
distance below 200, then queues `-1`, but writes no persistent unlock switch.
The pair opens transiently. A separate boar/event collision path destroys or
opens it without a key. It remains unimported until paired live state and the
destructive bypass can be represented without falsely persisting an unlock.

The lone `L7demo_dr` placement in D_MN07 room 6 uses `key_count > 0` plus an
exact local box to start a bridge cutscene, write switch `0x18`, and change
scene. It never decrements the key count, so it is explicitly not a keyed-door
consumer.

## Extracted event evidence

The exact object archives and decoded `event_list.dat` resources establish the
selected retail cuts:

| Archive SHA-256 | Resource SHA-256 | Established event behavior |
| --- | --- | --- |
| `95869eae5a21d1ba574eb16d73ac123420c21cc21daacc980e54e2cec239b20a` | `fd5570eca9bd29ee1b433236a10945872930fbf52c2508af9ff2c3f7ea9386fe` | L3 mini-boss front event contains `UNLOCK` before `OPEN`/`CHG_SCENE` |
| `afb1acd89b8b5cd6e262b6d362091c24a88687432528bc9b0c0a3c30330f5ea9` | `7de6bfac10e3ca6c3f6bc88a83815972d3397fd3488b067398cdd8cb0ea0cce4` | L7 mini-boss front event contains `UNLOCK` |
| `6f5da09c51603167299090a531be90a7c8eec3efa1c5698c74ce11032a82ea03` | `b079b8b284208582d9a37b50bd94f13400530abca75db0771147a646a8d83627` | L8 mini-boss front event contains `UNLOCK` |
| `e30c1779bf87fccd1061d0a151ebc0a35c5b225c18b2b55d451dfd7542e7de4f` | `8676effbd561ba65f8e4a8b9493aa6b60072d40f72a8e240b2ffa9c5550b40fa` | `KEY_JAIL_00` human/wolf variants contain `UNLOCK` before `OPEN` |
| `c07fa3cbc3af9c5444da2015478de4468af604f8568aa170d505b958babe7c89` | `3bff3ce52a0c1660d5ccf0bdcae24b672e50013317b3469698c51e32336c159a` | `KEY_JAIL_01` human/wolf variants contain `UNLOCK` before `OPEN` |
| `419fbb434b757c989db1d34b414280389d2361782708e78281744bb0792094b6` | `2184efba5db7b458f01c50534e29ba072fcb58be5e3b6df8f92e35b758726440` | Lakebed boss-shutter front event contains `UNLOCK` before `OPEN` |
| `f68b5c754d2ccefc5f80f74e05fb659898e2f12c568260d4ad39237a02ce63a9` | `c8684156665423d1a133dc0b102098d8ec3be838dd6abe117dbafedf0144ab83` | `KOKI_GATE_OPEN00` contains `UNLOCK` before `OPEN` |
| `a3700cf327a2469195fa93860b69dbada71cbff64bb846b0ee7cf0d932bb4e92` | `95582d74d858aeb5b01a9f1beb6c0c1bd6761b619b75f57d1d60d906f73ea856` | `RIDER_GATE_OPEN00` contains `UNLOCK` |

## Executable v10 boundary

For exact GZ2E01 content, `extracted-world-facts/v10` imports:

- the six front-side option-2 mini-boss placements as distinct first-open and
  already-unlocked `Door` branches joined to their unique SCLS destinations;
- all five internally checked retail key shutters, including separate zero,
  normal-range, and high-raw-count Lakebed boss-shutter outcomes; and
- the three layer-sensitive, memory-switch-backed type-0 Koki-gate placements.

The hard predicates resolve the source stage/room/layer, current-stage switch,
small-key count, and boss-key bit from exact raw backing. Effects retain switch
writes, queued-count-equivalent decrements, and scene destinations separately.
Interaction geometry, resource/event completion, keyhole behavior, collision
release, restart handling, and post-unlock pushing remain named feasibility
obligations.

V10 also separates transition association by kind: encoded-map and door
transitions must be referenced exactly once by an encoded SCLS record, while
in-room actor-driven actions must not be attached to a fabricated exit.

For L5 boss doors, v10 replaces the former prose-only interaction boundary
with placement-derived typed observations: a yaw-oriented actor-local
`checkArea` rectangle, a positive-local-Z front plane, and a shortest-circular
binary-angle facing obligation. The loaded actor/event/collision/scene-change
phase remains unresolved, and L1's wolf-only attention-position plus narrower
current-position compound test remains explicit rather than being approximated
as a single player-origin rectangle.

The importer intentionally excludes rider gates, caravan gates, externally
switched `vshuter`, absent/non-memory switches, Koki-gate types without exact
retail evidence, the guard-only bridge demo, and the zero-placement generic
`bdoor` family. Those exclusions are tested. Adding them requires modeling the
specific event-bit, switch-domain, paired/transient, destructive, or placement
semantics described above; actor-name similarity is not sufficient.

This audit closes the exact-build keyed family census. Live temporal variation
for other `DOOR20` configurations, generic collision/navigation proof, and
cross-build equivalence remain separate tasks.
