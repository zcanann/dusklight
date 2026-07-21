# Lanayru spirit and Vessel of Light source audit

Status: partial source audit. The committed C++ establishes the generic actor,
message, item, and save semantics below. `orig/` is currently empty, so the
retail stage placement and `Seirei1` message-flow asset cannot yet be extracted.
Those missing resources are required before claiming an exact build-specific
appearance/grant predicate.

## What is established

The light-spirit actor does not delete itself from an event-bit check. Its
`isDelete()` always returns false; actor existence therefore begins with a
placement on the loaded stage/room/layer and successful resource creation, not
with the friendly “received vessel” flag
([actor source](../../src/d/actor/d_a_npc_seirei.cpp#L230),
[delete predicate](../../src/d/actor/d_a_npc_seirei.cpp#L336)). The actor reads
the following placement data:

- low parameter byte: spirit type `0`, `1`, `2`, or fallback `3`;
- bits `12..19`: room switch used for attention/interaction;
- bit `8`, inverted: whether the actor supplies the first-meeting Do-button
  prompt;
- high parameter nibble: particle-only (`0`) versus modeled spirit (nonzero);
- home angle X: message-flow node, with `0xffff` meaning none.

These decoders are explicit in the actor and header
([type/high nibble](../../src/d/actor/d_a_npc_seirei.cpp#L230),
[parameter accessors](../../include/d/actor/d_a_npc_seirei.h#L91)).

For the particle/first-meeting form, the decoded room switch must not be `0xff`
and must be on in the actor's current room before the actor advertises the
`SPEAK` attention flag
([attention gate](../../src/d/actor/d_a_npc_seirei.cpp#L388)). The visible
particle emission uses the same switch check. A normal manual first-meeting
prompt additionally requires particle form, the type-specific first-meeting
predicate, an enabled Do-button check, and the engine's speak test
([wait action](../../src/d/actor/d_a_npc_seirei.cpp#L732)). Type `2` checks that
event-bit index `67` (`M_034`, raw save label `0x0820`) is false; notably, it does
not check `F_0615` in this C++ path
([type checks](../../src/d/actor/d_a_npc_seirei.cpp#L661),
[raw labels](../../include/d/d_save_bit_labels.inc#L740)). Whether the Lanayru
vessel placement is type `2`, and its exact parameter/switch/layer values, still
has to come from the stage asset.

Talking is not itself the item grant. The actor starts its placement-selected
message flow. Only after that flow returns event ID `1` with an item number does
the actor create the presentation item actor and switch to the shared
`DEFAULT_GETITEM` event
([talk handoff](../../src/d/actor/d_a_npc_seirei.cpp#L761)). Thus the model needs
separate states for actor loaded, attention/speak eligible, talk started, flow
reaching its item event, presentation actor executing, and generic get-item
completion.

The actual Lanayru Vessel storage is not an ordinary inventory slot. Item
`DROP_CONTAINER03` sets bit `1 << 2` (`0x04`) in
`dSv_player_info_c::mLightDrop.mLightDropGetFlag`; the three vessel indices are
Faron `0`, Eldin `1`, and Lanayru `2`
([item writer](../../src/d/d_item.cpp#L1072),
[indices](../../include/d/d_save.h#L106),
[backing layout](../../include/d/d_save.h#L438),
[bit operation](../../src/d/d_save.cpp#L936)). The message-flow `Item Set`
handler has the same special-case write and also changes a meter-only display
state
([flow writer](../../src/d/d_msg_flow.cpp#L2023)). The corresponding item check
reads the vessel bit, so these are genuine alternate writers of the same backing
store rather than separate conceptual items
([item reader](../../src/d/d_item.cpp#L1739)).

`F_0615` is a different persistent event bit with raw save label `0x4b04`
([label](../../include/d/d_save_bit_labels.inc#L504)). Neither generic Vessel
item writer above sets it. If the normal Lanayru flow sets `F_0615`, that is a
separate message-flow event-node write whose ordering must be extracted from the
missing `Seirei1` flow asset. The planner must not derive `F_0615` merely from
the Vessel ownership bit, or derive the Vessel bit merely from `F_0615`.

Lanayru Spring is stage `F_SP115`, room `1`; known loader metadata exposes spawns
`0, 1, 20, 21, 22, 23, 100`
([map metadata](../../include/dusk/map_loader_definitions.h#L222)). This identifies
the candidate map, but not the retail actor layer or activation switch.

## Current exact precondition answer

With the currently committed evidence, the strongest honest predicate is:

```text
placement NPC_SEIREI is loaded on its authored F_SP115 room-1 layer
AND its resource archive finishes loading
AND placement high-nibble == 0                    # particle/talk form
AND placement switch != 0xff
AND room_switch(current_room, placement.switch) == true
AND (if manual first-meeting prompt is required:
       placement type's first-meeting bit is false
       AND placement bit 8 == 0
       AND engine speak-range/facing/control test succeeds)
AND placement-selected Seirei1 flow reaches event ID 1
AND that event's item ID == DROP_CONTAINER03 (0xA3)
AND the presentation item actor successfully executes
```

The resulting grant writes `player.light_drop.get_flags |= 0x04`. The exact
layer, placement parameter word, room-switch number, flow node, flow branches,
and any `F_0615` write remain unknown until an original-data directory containing
`F_SP115` and `Seirei1` resources is supplied and content-identified.

## Planner records still needed

1. Import the exact `F_SP115` room-1 actor placement on every supported build and
   bind its layer, parameter word, home angle X, position, and resource archive.
2. Decode the referenced `Seirei1` flow node, retaining every branch and ordered
   event node, especially event ID `1`, item `0xA3`, and event-bit writes.
3. Represent the room switch as local stage/room backing, the Vessel flag as
   persistent player backing byte `mLightDrop + 4` mask `0x04`, and `F_0615` as
   persistent event raw label `0x4b04`.
4. Keep appearance, speak eligibility, talk, flow item request, presentation,
   generic get-item, Vessel write, and story-bit write as distinct transitions.
5. Trace normal and alternate entrances against the exact loaded layer and live
   actor placement; absent placement/layer evidence must remain unknown.
