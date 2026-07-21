# Lanayru spirit and Vessel of Light audit

Status: exact for the supplied GZ2E01 original-data tree; other builds remain
unverified. Actor placement, spatial switch writer, layer selection, message
flow, item presentation, persistent Vessel backing, and post-grant story writes
are separate predicates and transitions.

## Exact GZ2E01 placement and layer

`orig/GZ2E01/files/res/Stage/F_SP115/R01_00.arc` has SHA-256
`c904e517476e46884cd719930d45129a480cefd6405f05e48fa0cb43737db4c8`.
Its `room.dzr` contains this layer-`d` (`13`) actor:

```text
ACTd[0]
name       Seirei
parameters 0x0000c102
position   (2.677870, 753.708252, -620.950562)
angle      (21, -32768, 0)
set_id     0xffff
raw        53656972656900000000c102402b6239443c6d54c41b3cd6001580000000ffff
```

The actor's committed decoders make those bytes semantic
([actor creation](../../src/d/actor/d_a_npc_seirei.cpp),
[parameter accessors](../../include/d/actor/d_a_npc_seirei.h)):

- low byte `0x02`: spirit type 2 (Lanayru);
- bits `12..19`: switch ID `0x0c`;
- bit 8 is set: the optional first-meeting Do-status helper is disabled;
- high parameter nibble `0`: particle form, with no modeled spirit heap;
- home angle X `21`: message flow 21.

The placement is loaded only on layer 13. With a default scene layer, Lake
Hylia room 1 selects twilight layer 14 while player-status-B byte `0x31`, mask
`0x04` (`mDarkClearLevelFlag[Lanayru]`), is clear. If `M_032` is also set, the
shared layer selector replaces 14 with 13 for `F_SP115`; `M_032` is event byte
`0x08`, mask `0x80` (raw label `0x0880`) and means the Zora river ice was melted
with the magma rock
([layer selection](../../src/d/d_com_inf_game.cpp),
[raw label](../../include/d/d_save_bit_labels.inc)). An explicitly forced layer
13 can load the same placement without reproducing that normal derivation, so
the planner records both `selected layer == 13` and its usual writer provenance.

The actor's `isDelete()` is always false. Vessel ownership, `F_0615`, and
`M_034` do not remove the placement. Successful `Seirei1` resource loading is
still required; type 2 loads that archive and its event list
([resource table](../../src/d/actor/d_a_npc_seirei.cpp)).

## What makes the particle visible and speak-eligible

The same room archive contains this scaled layer-13 actor:

```text
SCOd[0]
name       SwAreaC
parameters 0xff0cff0c
position   (19.151329, 208.822067, -1908.488037)
angle      (255, 0, 0)
scale_raw  (0x46, 0x46, 0x46)
raw        5377417265614300ff0cff0c4199b1ec4350d273c4ee8f9e00ff00000000ffff464646ff
```

Its type is 0, shape is 3 (cylinder), scale mode is 0, output switch is `0x0c`,
secondary switch is `0xff`, condition is `0xff`, and event ID is `0xff`. The raw
scale bytes become actor scale `7.0`: the accepted volume is horizontal distance
less than `730` and vertical delta strictly between `-100` and `700`. Condition
`0xff` adds no human/wolf, mount, or carried-flame restriction. While the player
is inside this cylinder the actor turns switch `0x0c` on; outside it turns the
switch off. Type 0 also clears a preexisting `0x0c` during actor creation
([switch-area execution](../../src/d/actor/d_a_swc00.cpp)).

Despite the API taking the actor's room number, this is **not room-local state**.
`dSv_info_c::{on,off,is}Switch` routes IDs below `0x80` to the live
`dSv_memory_c` stage bank. Therefore `0x0c` is F_SP115 stage memory,
`mSwitch[0]` bit 12 (raw `dSv_memBit_c` byte `0x0a`, mask `0x10`), serialized
through that stage's save bank. The placement is room-local; its output is not.
This is exactly the backing/semantic distinction the planner must preserve
([switch dispatch](../../src/d/d_save.cpp),
[memory layout](../../include/d/d_save.h)).

The spirit independently reads that same stage-memory switch. When `0x0c` is
on, particle form emits the two spirit particle effects and advertises the
`SPEAK` attention flag; when it is off, both disappear
([attention and particles](../../src/d/actor/d_a_npc_seirei.cpp)). The exact
visible/eligible predicate is therefore:

```text
scene.stage == F_SP115
AND scene.room == 1
AND selected_layer == 13
AND ACTd[0] Seirei resource creation completed
AND actor.parameters == 0x0000c102
AND SCOd[0] SwAreaC resource creation completed
AND player_position is inside SCOd[0] SwAreaC's decoded cylinder
AND stage_memory(F_SP115).switch[0x0c] == true
AND no incompatible event currently owns interaction control
```

The cylinder is the normal writer prerequisite, not a hidden predicate in the
spirit: the spirit itself reads only switch `0x0c`. A transferred switch can
therefore produce a transient eligible state, but `SwAreaC` clears it on create
or on its next outside-volume execution. Actual talk additionally needs the
engine attention-distance/facing/input checks and usable player/event control.
The normal GZ2E01 type-2 path has no form, mount, or temporary-bit prerequisite.
`M_034` (raw `0x0820`) is consulted by `chkFirstMeeting()`, but the placed
actor's bit-8 setting makes that predicate irrelevant to its disabled Do-status
helper. `F_0615` is not an appearance or attention guard.

## Exact flow 21

The stage `STAG` record selects message group 8. In the supplied US-English
build, `orig/GZ2E01/files/res/Msgus/bmgres8.arc` has SHA-256
`2562ae9662648e71b8f30a5682dbc440dae3a7de55782bbd5992e4192e38e2cb`.
Its `zel_08.bmg` maps flow label 21 to node 321. The relevant graph is:

```text
321 query001(saveBitLabels[615] == F_0615)
  F_0615 set     -> message 272 -> message 273 -> end
  F_0615 clear   -> 314 query022(item 0xa3 / DROP_CONTAINER03 owned)
    item absent  -> messages 266, 267, 268
                 -> event008(event_id=1, item_id=0xa3) -> end
    item present -> messages 269, 270, 271
                 -> event000(saveBitLabels[615])
                 -> event017(item=0xa3, count=1)
                 -> event014(save-switch=105) -> end
```

The branch result order follows the C++ query implementations: `query001`
returns 1 when its event bit is clear, and `query022` returns 1 when its item is
absent ([flow queries and events](../../src/d/d_msg_flow.cpp)).

`event008` does not grant the item. It only leaves event ID 1 and item ID `0xa3`
in the live flow state. The spirit actor reads those values, creates the
presentation item actor, and changes to `DEFAULT_GETITEM`. Generic item
completion then writes the Lanayru Vessel backing. Because the actor sets its
post-presentation latch, its next wait orders a speak event; the follow-up flow
now takes the item-present branch, sets `F_0615`, idempotently reasserts item
`0xa3`, and sets save-switch 105. Those are ordered effects, not one milestone.

## Backing stores and writers

The Vessel is not an ordinary inventory slot. `DROP_CONTAINER03` is the bit
`1 << LANAYRU_VESSEL`, or mask `0x04`, in
`dSv_player_info_c::mLightDrop.mLightDropGetFlag`. Both the generic item writer
and message-flow `event017` write that bit
([generic item writer](../../src/d/d_item.cpp),
[flow item writer](../../src/d/d_msg_flow.cpp),
[save backing](../../src/d/d_save.cpp)). Item checks read the same bit.

`F_0615` is a different persistent event bit with raw label `0x4b04`. Flow 21's
item-present branch writes it through `event000`; neither generic item grant nor
the Vessel backing setter implies it. This distinction explains the otherwise
surprising intermediate state `Vessel owned && F_0615 clear` during normal
presentation and makes item injection/duplication theorycrafting representable.

The remaining exact GZ2E01 backing coordinates are:

| Meaning | Backing coordinate |
| --- | --- |
| Lanayru twilight cleared | player status B byte `0x31`, mask `0x04` |
| Vessel owned | player light-drop byte `0x118`, mask `0x04` |
| Lanayru tear count | player light-drop byte `0x116` |
| `M_032` | event byte `0x08`, mask `0x80` |
| `F_0615` | event byte `0x4b`, mask `0x04` |
| activation switch `0x0c` | live/serialized F_SP115 `dSv_memBit_c::mSwitch[0]` bit 12; component byte `0x0a`, mask `0x10` |
| post-flow switch 105 | same F_SP115 stage bank, `mSwitch[3]` bit 9; component byte `0x16`, mask `0x02` |

Message-flow `event014(0, 105)` uses `dComIfGs_onSaveSwitch(105)`, so the final
switch is in the same live F_SP115 stage-memory component as activation switch
`0x0c`; it is not part of the global event-bit array or room-local zone memory.

Tear count is another store again: `mLightDropNum[LANAYRU_VESSEL]`. Individual
tear actors increment the count through `dComIfGs_setLightDropNum`, the
`KYTAG04` controller authors the required-count runtime value and compares it
against the current dark-area count, message-flow query 027 reads it, and meter,
map, player-effect, save-HIO, and twilight-end consumers display or react to it.
Vessel ownership must not be inferred from a nonzero tear count, nor vice versa.

The source-reference audit has one gameplay writer for collected-tear count:
`daObjDrop_c::dropGet` marks the tear's treasure bit, reads the current dark-area
byte, and writes back `count + 1`. Initialization clears all four count bytes.
The Vessel flag has two normal gameplay writers—generic item `0xa3` and
message-flow event017—plus initialization/explicit setter APIs and debug-only
editors. Its gameplay consumers are the item-owned query, tear pickup range,
the twilight Kargarok route, field-map visibility, and HUD/effect code. Count
consumers are `daObjDrop` completion, `KYTAG04`, message query027, field-map
visibility, and HUD/player effects. `d_save_HIO` observes both for debugging.
This list is derived from every committed reference to the accessor and backing
symbols; new direct-memory users must be added when discovered.

## Tear completion is a later controller flow

GZ2E01 `F_SP115/STG_00.arc` (SHA-256
`a7783343c82477903b18dc0a63c7d41235cf6d1709ef2298f74df19345a125c6`)
contains common `ACTR[0]` actor `DK_tag` with parameters `0xff0d0100`, angle Z
`0xff10` (low byte `16`), and this raw record:

```text
444b5f7461670000ff0d0100c7a74c02c63062234778bac700000000ff10ffff
```

`KYTAG04` decodes that as type 1, exit 0, switch 13, and required count 16. It
writes the required count to session/play storage, watches Lanayru tear count,
and changes through SCLS exit 0 after switch 13 is authored. Exit 0 is
`F_SP115`, room 1, point 20, layer 8. Loading point 20 makes `d_s_play` set
`mDarkClearLevelFlag[2]`, ending Lanayru twilight. These are later transitions;
none is a prerequisite for the initial layer-13 spirit interaction.

## Route-planner implications

- Alternate entrances and wrong-state respawns work only if their selected
  layer is 13 and they place Link inside or allow Link to reach the switch
  cylinder.
- A forced wrong layer can reach the room while omitting both `Seirei` and
  `SwAreaC`; location alone is insufficient.
- Directly transferring F_SP115 stage switch `0x0c` can make the spirit visible,
  but the type-0 switch-area actor clears it during creation if already set, or
  on its next outside-volume execution. A hypothetical transfer must account
  for that ordered writer or prevent/bypass the actor's execution.
- Transferring only the Vessel bit changes flow 21 to the item-present branch;
  it does not create the actor, select layer 13, or satisfy spatial interaction.
- Transferring only `F_0615` makes flow 21 take its completed dialogue branch;
  it does not grant the Vessel.
- Actor/resource failure after the presentation request must preserve an
  explicit pending operation or unknown suffix; the solver may not assume the
  generic grant occurred.

The planner acceptance fixture follows this exact ordering and proves that
appearance, interaction, item request, presentation, Vessel write, `F_0615`, and
post-flow switch state can be queried separately. Only GZ2E01 has an exact raw
predicate today; each additional build/language must be content-identified and
compared rather than silently mapped to these offsets and node numbers.
