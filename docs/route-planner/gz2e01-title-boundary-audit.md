# GZ2E01 title/reset boundary audit

This audit covers the successful reset-to-opening prefix and the confirmed
opening-phase file-0 initializer for the exact GameCube USA revision-0
executable (`GZ2E01`). It does not yet claim complete title, file-select, void,
or death behavior.

## Exact executable evidence

The following functions were retained with the planner-owned
`binary-function-evidence/v1` extractor from the registered GZ2E01 `main.dol`
and symbol table:

| Function | VA | Size | Code SHA-256 | Artifact SHA-256 |
| --- | ---: | ---: | --- | --- |
| `dComIfG_changeOpeningScene__FP11scene_classs` | `0x8002cc54` | `0xf0` | `0b5c465a32ffb343d9863e04970f5c2621a5bb0b854efc974708fb0229828a41` | `658f63b09b0f43dcb5b2662dbbf140de889fe19374dac8ccee32d9545ac2d781` |
| `dComIfG_resetToOpening__FP11scene_class` | `0x8002cd44` | `0x74` | `3cc637771d531950401a332a83b90296df2b5aa9bec6cc292ad5546fec23df30` | `bde63a102b6502e418e5a8c53cff364f66f6510420a7316a492664ab7530e28d` |
| `phase_4__FP9dScnPly_c` | `0x8025a654` | `0x3a0` | `5e116171d689fcf368218490f24009dd176205648fd30b697bdab3a7efb179aa` | `caf6f662835287e2c74e341b2771e142c8b0a1dd6da7745775a01f1a36cb62cc` |
| `init__10dSv_info_cFv` | `0x80034fcc` | `0x50` | `5c80b3dba87ae8f968b5e4620f0872d4355358debc63d5556adba4b8d3d4338d` | `433224e88c9c58df6d5abd49863e2a871a965f2806288e1d19fd36f1e267d93b` |
| `init__10dSv_save_cFv` | `0x8003501c` | `0x8c` | `e405d830e4f445c950fb158ddf8f6107430524a2708d82bd1b31c7e13e804d48` | `a9953253f543fbdc9d0998e6f369fb2f0bac45b411c44baee5ff9fd34fccda9b` |
| `init__12dSv_player_cFv` | `0x800346a4` | `0xac` | `668f452c16c5ed413535588b00c5a497b236a29f7e52f55c521b58e179968766` | `0bc0b6246b3a6cad9a8a0409ef59358fa544632ac5884b27008a3e5dd4db185b` |
| `setInitEventBit__Fv` | `0x80035c88` | `0x4` | `f332ea5b5437103cbb6f1508679da89eec9288ad775c96c439a17fccabe3de8e` | `c40daaee608a8afd5c471d54a1a87efe7eb42695036729215a3fa413d256892f` |
| `init__25dSv_player_return_place_cFv` | `0x80032cc8` | `0x54` | `252007ca2690e54e6a13019527739c4e55dff0f1ac1e7ec6ff8b1d425ed6ab87` | `0eeb93826008824d6810499ce61ec1c8e8065c7a06c8a9576022b76532f75917` |
| `dComIfGs_setSelectEquipSword__FUc` | `0x8002eec0` | `0xd4` | `b0cdfc30b3f91a906cf4c8066f8eb5ec7055df50de7ade590c5c721ea0732761` | `1d014bd60aa88951beb555a13853be0068f91790989639909bcff8a088decd9e` |
| `dComIfGs_setSelectEquipShield__FUc` | `0x8002ef94` | `0xac` | `beeb64d1fa6897f83de2674e9053189416486ca4066c39d1efb4e647bf7c7e14` | `7a7920012416bdf116d20be436514da59bf00da2e6cbab28dcc0842e33078a23` |

The semantic audit uses these source-family snapshots:

| Source | SHA-256 |
| --- | --- |
| `src/d/d_com_inf_game.cpp` | `b9b37aed0b76eef2d27b35a2ece6ee077086a970f98d18936a83649303f15761` |
| `src/d/d_s_play.cpp` | `c8f30a83c45d6c42078945b09f6e4e3459c832184e641ff442fa7d0e49258077` |
| `src/d/actor/d_a_title.cpp` | `39378bcbc78e5ffae3287f127cc48cd2c22e18723cf31cfeb5bd84a2becdc4cb` |
| `src/d/d_s_name.cpp` | `f095894aabc198c068ee0ac9872f6c277c0e035b36c4d29d1f896e7c2eb0fe4b` |
| `src/d/d_file_select.cpp` | `aee1cb134ec92953fd04dc321f4dae5f5c98ed1d2e766d1306a70d932294eb0d` |
| `src/d/d_save.cpp` | `7e6f09aa36af30932e8ce64423284f885ed0b4e632b22f18d6f0a6b4d104b453` |
| `src/d/d_meter2_info.cpp` | `73b58242c7f742f4ac46ddda5f5c8b39d24e73beebffaaa5aa2d8d011a641b6e` |

The source snapshots explain the retained machine code but are not treated as
additional exact retail identities.

## Confirmed successful prefix

On GCN, `dComIfG_resetToOpening` returns without changing scenes when any of
these conditions holds:

- return-to-menu is active;
- reset is not requested; or
- the fader status is `2`.

When all three guards permit the action,
`dComIfG_changeOpeningScene(..., PROC_OPENING_SCENE)` performs the confirmed
state-changing prefix:

1. disable the existing next-stage enable state;
2. schedule stage `F_SP102`, start point `100`, room `0`, layer `10`;
3. set the restart-room parameter to `0`; and
4. request and re-request `PROC_OPENING_SCENE`.

Audio and fade operations are real but are not route-relevant backing effects
in the current model.

## Why the process/map distinction matters

The scheduled `F_SP102` target is not a completed world transition. The opening
scene is a top-level process which later creates the title actor and performs
additional initialization. Treating its pending stage request as Link already
standing in `F_SP102` would authorize ordinary map actions during title/file
select.

Execution-environment schema v7 therefore carries an explicit execution
context:

- `world` means the stored map location is active;
- `process` names the non-world owner and may retain a pending world-load
  request; and
- `unknown` preserves uncertainty rather than choosing either state.

Location, active-player, live-world actor, actor-instance component,
spatial-obligation, and current stage/room binding reads resolve unknown outside
`world`. Global, runtime-file, and session stores remain readable because title
and file-select logic can legitimately operate on them. `SetLocation` and the
dynamic return-place load explicitly restore the `world` context. The reset
transition instead retains the last world location for inspection while
entering `PROC_OPENING_SCENE` with the pending F_SP102 target. State diffs keep
both execution contexts and a validated changed marker, so the boundary stays
visible even when retained map coordinates do not change.

The exact catalog is emitted with:

```text
route-planner compile-title-boundary-mechanics \
  --content-identity CONTENT.json \
  --runtime-configuration RUNTIME.json \
  --output MECHANICS.json
```

It is restricted to the registered GZ2E01/English exact context. Its reset
guards are ordinary component-field predicates over `reset-control`; missing
observations make activation unknown.

## Confirmed opening/file-0 initialization

The catalog also contains two mutually exclusive opening-phase-4 transitions.
Both require all of the following independently observable state:

- process `PROC_OPENING_SCENE`;
- pending stage `F_SP102`, room `0`, layer `10`, spawn `100`; and
- planner process-scheduler component `opening-process-control` at `phase_4`.

If the active runtime is already title-origin file 0, the direct initializer
uses it. Otherwise, the enter-and-initialize variant atomically ends the
incoming runtime lifetime, derives a fresh runtime ID from it, rekeys every
runtime-owned live component and serialized stage store to that new lifetime,
and gives it `title_file_0` origin, memory-only backing, and slots 1–3 as legal
serialization targets. Session/process components, unrelated inactive
runtimes, and sealed physical-slot images remain independent. This is a
lifetime transition, not a fictional physical slot 0.

The explicit scheduler guard matters: merely requesting F_SP102 does not prove
that opening phases 0–3 and their resource work have reached phase 4. The
transition applies the projected `dComIfGs_init()` effects and later phase-4
writes, then completes the pending load without changing execution to `world`.
It publishes these exact final values in the canonical snapshot components:

- max life `15`, life `12`, and rupees `0`;
- all 24 inventory slots and all selected/mixed item slots empty (`0xff`);
- Hero's Clothes (`0x2f`), Ordon Sword (`0x28`), and Hylian Shield (`0x2c`)
  selected;
- bomb and bottle counts zero, bomb capacities `30`, `15`, and `10`, and all
  projected first-item acquisition bytes zero;
- the Ordon Sword collection bit (`collect_item_bits[1] & 0x01`) and Hylian
  Shield collection bit (`collect_item_bits[2] & 0x04`) set by their equipment
  setters;
- event bit `0x0601` (Epona tamed) set; and
- player return place `F_SP108`, room `1`, player status/start point `0`.

The equipment setters receive `offItemBit = false`; they do not grant
first-item/acquisition bits, but the sword and shield setters independently
write their collection masks. The exact GZ2E01 `setInitEventBit`
body is an immediate return, so the persistent event payload is 256 zero bytes
before phase 4 changes only byte `6` with mask `1`.

`dSv_info_c::init()` resets savedata, live stage memory, dungeon memory, zones,
and temporary event state. The current native projection can replace the full
known payloads for persistent event registers, five projected light-drop bytes,
temporary event registers, live `dSv_memBit_c`, inventory/resources, and return
place. The two unwritten tail-padding bytes of the 0x20 stage-memory payload
remain unknown. Friendly label-only event/temporary/dungeon/room-switch views are
invalidated rather than made inconsistent with their backing bytes. Existing
serialized `DungeonMemory` stores owned by the active runtime are also
invalidated because their representation may not expose the same raw layout.
Unrelated inactive runtime stores and sealed physical-slot images are not
touched.

The initializer preserves `mRestart`, `mTurnRestart`, `mDataNum`, `mNewFile`,
`mNoFile`, and time controls because `dSv_info_c::init()` does not write them.
The canonical `restart` and `runtime-file.header` components are therefore
unchanged. Option-pointer, transform-status, names/configuration, minigame,
all 32 saved stage banks, the secondary visited-room banks, dungeon working
memory, and zone-array fields are not all projected by current native snapshots;
this milestone does not claim those absent fields as modeled defaults.

The lifetime handoff is attached to phase 4 rather than the earlier reset
request: requesting `PROC_OPENING_SCENE` does not itself prove that
`dComIfGs_init()` executed. Unknown runtime origin also remains unknown instead
of being silently classified as file 0.

## Title input and normal file-select creation

The next source-audited suffix is represented as three independently guarded
transitions rather than one opaque jump:

1. While `PROC_OPENING_SCENE` remains active and the title actor is observed in
   `key_wait`, an observed A or Start trigger advances its control state to
   `next_scene`. Reset-active or overlap-peek state blocks the actor exactly as
   in `daTitle_c::Execute`.
2. The next title update submits `fopScnM_ChangeReq(..., PROC_NAME_SCENE, ...)`
   and records `scene_requested`. It deliberately leaves the execution context
   at `PROC_OPENING_SCENE`: a request is not evidence that the process manager
   completed the handoff.
3. Only an independently observed active `PROC_NAME_SCENE` at
   `create_file_select` authorizes the route-relevant projection of
   `dFile_select_c::_create` and `dScnName_c::create`.

The third transition runs the same audited `dComIfGs_init()` projection again,
then writes `mNewFile = 0` and, for the normal GCN name scene, `mNoFile = 0`.
Consequently it clears the title-opening Epona bit and sword/shield collection
masks, restores base Ordon clothes with no sword or shield selected, and resets
the projected return place, stage memory, event bytes, temporary bytes, and
active-runtime serialized dungeon stores. It does not begin another runtime
lifetime, change `mDataNum`, mutate the restart record, touch unrelated inactive
stores, or alter any sealed physical-slot image.

The title and name-scene control components are scheduler/actor observations,
not invented persistent flags. A missing phase, trigger, reset state, overlap
state, or active-process observation makes the corresponding transition
unknown or blocked. This preserves the distinction between an input, a scene
request, process activation, and completed file-select initialization.

## File-select decision suffix

The source-audited blank-slot, existing-slot Start, and no-card decisions are now
separate transitions. Blank selection writes `mNewFile = 128` and the zero-based
`mDataNum` without loading or populating a slot. No-card acceptance initializes
three custom session-buffer stores, copies entry 0's persistent projection into
the still-memory-only live runtime, and writes `mNoFile = 1`, `mDataNum = 0`
without executing the blank-slot `mNewFile` writer.

Existing-slot Start derives the persistent-file identity and complete manifest
from the selected populated physical slot, retires the active file-0 lifetime,
and restores a fresh card-backed runtime. Header, restart, and temporary-event
metadata are explicit non-card carries; omitted runtime metadata dies. Exact
GZ2E01 DOL artifacts now seal `card_to_memory` and `setLineUpItem`. The
conditional 12-life minimum, dungeon-6 key reset, both hookshot layout repairs,
exact lineup rebuild, saved-vibration application, and return-place-derived
display stage now execute as typed operations after the backing projection. See
`gz2e01-file-select-branches.md`.

After independently observed `selection_end`, the next source-audited actions
remain process requests rather than world activation. A new file records the
fixed pending F_SP108/room 1/spawn 21/layer 13 destination. An existing file
reads stage, room, and player-status/spawn from its restored return-place
component with layer -1. Both retain active `PROC_NAME_SCENE`, record the
requested `PROC_PLAY_SCENE`, and leave the prior world location non-traversable.

That member-level distinction is central to file-0 and Back in Time reasoning.
Future projection coverage must continue adding ordered writers and explicitly
preserved members rather than approximating the title transition as “clear
all.”

Still open:

- finish name confirmation and successful-save branches;
- audit void and death restart selection, including their special-stage and
  boss-room branches; and
- produce traces that distinguish pending scene requests from completed world
  loads.
