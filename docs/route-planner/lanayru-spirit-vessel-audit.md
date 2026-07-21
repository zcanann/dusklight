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
- bits `12..19`: current-room switch `0x0c`;
- bit 8 is set: the optional first-meeting Do-status helper is disabled;
- high parameter nibble `0`: particle form, with no modeled spirit heap;
- home angle X `21`: message flow 21.

The placement is loaded only on layer 13. With a default scene layer, Lake
Hylia/Lanayru twilight first selects layer 14. If `M_032` is set, the shared
layer selector replaces 14 with 13 for `F_SP115`; `M_032` has raw event label
`0x0880` and means the Zora river ice was melted with the magma rock
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

Its type is 0, output switch is `0x0c`, secondary switch is `0xff`, and event ID
is `0xff`. While the player is inside its box it turns current-room switch
`0x0c` on; outside it turns the switch off. It does not permanently award a
story flag ([switch-area execution](../../src/d/actor/d_a_swc00.cpp)).

The spirit independently reads that same current-room switch. When `0x0c` is
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
AND player_position is inside SCOd[0] SwAreaC's decoded box
AND room_switch(F_SP115, room 1, 0x0c) == true
AND no incompatible event currently owns interaction control
```

The final line is an engine scheduling/attention condition, not an authored
story bit. `M_034` (raw `0x0820`) is consulted by `chkFirstMeeting()` for type 2,
but the placed actor's bit-8 setting makes that predicate irrelevant to its
disabled Do-status helper. `F_0615` is not an appearance or attention guard.

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

Tear count is another store again: `mLightDropNum[LANAYRU_VESSEL]`. Individual
tear actors increment the count through `dComIfGs_setLightDropNum`, the
`KYTAG04` controller authors the required-count runtime value and compares it
against the current dark-area count, message-flow query 027 reads it, and meter,
map, player-effect, save-HIO, and twilight-end consumers display or react to it.
Vessel ownership must not be inferred from a nonzero tear count, nor vice versa.

## Route-planner implications

- Alternate entrances and wrong-state respawns work only if their selected
  layer is 13 and they place Link inside or allow Link to reach the switch box.
- A forced wrong layer can reach the room while omitting both `Seirei` and
  `SwAreaC`; location alone is insufficient.
- Directly transferring current-room switch `0x0c` can make the spirit visible,
  but the type-0 switch-area actor will clear it on its next outside-volume
  execution. A durable hypothetical transfer must also account for that writer.
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
