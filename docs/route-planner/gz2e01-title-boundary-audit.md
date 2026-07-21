# GZ2E01 title/reset boundary audit

This audit covers the successful reset-to-opening prefix for the exact
GameCube USA revision-0 executable (`GZ2E01`). It does not yet claim complete
title, file-select, void, or death behavior.

## Exact executable evidence

Two functions were retained with the planner-owned
`binary-function-evidence/v1` extractor from the registered GZ2E01 `main.dol`
and symbol table:

| Function | VA | Size | Code SHA-256 | Artifact SHA-256 |
| --- | ---: | ---: | --- | --- |
| `dComIfG_changeOpeningScene__FP11scene_classs` | `0x8002cc54` | `0xf0` | `0b5c465a32ffb343d9863e04970f5c2621a5bb0b854efc974708fb0229828a41` | `658f63b09b0f43dcb5b2662dbbf140de889fe19374dac8ccee32d9545ac2d781` |
| `dComIfG_resetToOpening__FP11scene_class` | `0x8002cd44` | `0x74` | `3cc637771d531950401a332a83b90296df2b5aa9bec6cc292ad5546fec23df30` | `bde63a102b6502e418e5a8c53cff364f66f6510420a7316a492664ab7530e28d` |

The semantic audit uses these source-family snapshots:

| Source | SHA-256 |
| --- | --- |
| `src/d/d_com_inf_game.cpp` | `b9b37aed0b76eef2d27b35a2ece6ee077086a970f98d18936a83649303f15761` |
| `src/d/d_s_play.cpp` | `c8f30a83c45d6c42078945b09f6e4e3459c832184e641ff442fa7d0e49258077` |
| `src/d/actor/d_a_title.cpp` | `39378bcbc78e5ffae3287f127cc48cd2c22e18723cf31cfeb5bd84a2becdc4cb` |
| `src/d/d_s_name.cpp` | `f095894aabc198c068ee0ac9872f6c277c0e035b36c4d29d1f896e7c2eb0fe4b` |
| `src/d/d_save.cpp` | `7e6f09aa36af30932e8ce64423284f885ed0b4e632b22f18d6f0a6b4d104b453` |

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

## Audited suffix, not yet executable

The later source chain is important but intentionally not folded into this
prefix:

- opening-scene phase 4 creates `PROC_TITLE`, calls `dComIfGs_init()`, and then
  writes several title baseline values;
- the title actor requests `PROC_NAME_SCENE` after title input;
- GCN name-scene creation sets `mNoFile` to `0`;
- file-select paths load a selected card image or construct new-file/no-card
  state, and successful saving updates `mDataNum` and `mNoFile`.

`dSv_info_c::init()` initializes `mSavedata`, live stage memory, dungeon
memory, zones, and temporary event state. Its body does not initialize
`mRestart`, `mTurnRestart`, `mDataNum`, `mNewFile`, `mNoFile`, or the time
fields. That member-level distinction is central to file-0 and Back in Time
reasoning, but the complete opening phase also performs other writes. A later
boundary program must therefore model the ordered writers and preserved
members rather than approximating the whole title transition as “clear all.”

Still open:

- compile the opening-phase initializer with exact per-component effects;
- model title input and name/file-select processes;
- model no-card, new-file, selected-slot, and successful-save branches;
- audit void and death restart selection, including their special-stage and
  boss-room branches; and
- produce traces that distinguish pending scene requests from completed world
  loads.
