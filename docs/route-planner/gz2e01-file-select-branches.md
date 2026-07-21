# GZ2E01 file-select branch audit

This audit records the first mutually exclusive file-select branches after the
normal title/name-scene initializer. It is deliberately narrower than a claim
that every save/load normalization rule is modeled. Its purpose is to keep
three different storage actions from collapsing into a friendly but incorrect
`choose file` edge.

The audited source artifacts are:

- `src/d/d_file_select.cpp`, SHA-256
  `aee1cb134ec92953fd04dc321f4dae5f5c98ed1d2e766d1306a70d932294eb0d`;
- `src/d/d_s_name.cpp`, SHA-256
  `f095894aabc198c068ee0ac9872f6c277c0e035b36c4d29d1f896e7c2eb0fe4b`;
- `src/d/d_save.cpp`, SHA-256
  `7e6f09aa36af30932e8ce64423284f885ed0b4e632b22f18d6f0a6b4d104b453`;
  and
- `src/d/d_com_inf_game.cpp`, SHA-256
  `b9b37aed0b76eef2d27b35a2ece6ee077086a970f98d18936a83649303f15761`.

These are exact current source-family artifacts for the GZ2E01 audit. Binary
function-range extraction remains required before promoting source-only details
to exact retail-DOL evidence.

## Storage domains

The following objects must stay distinct:

1. `dFile_select_c::mSaveData` is a process/session buffer containing three
   quest-log entries. Loading the memory-card game file populates this buffer;
   it does not load any selected entry into live `dSv_info_c` state.
2. The active `dSv_info_c`/`dSv_save_c` state is the current runtime file. The
   file-select constructor has already run `dComIfGs_init()` on it.
3. Save slots 1–3 are sealed persistent images. The zero-based source index
   `mSelectNum`/`mDataNum` maps to user-facing slot `index + 1`.
4. `mDataNum`, `mNewFile`, and `mNoFile` are live header/control members. They
   are not copied by `dSv_info_c::card_to_memory`, whose `memcpy` covers only
   `dSv_save_c`.
5. File-select actor/process phases, selected index, menu command, and the
   `mIsDataNew`/`mIsNoData` classification are observed transient control state,
   not persistent game flags.

`setInitSaveData()` loops over all three entries of `mSaveData`, calls
`initdata_to_card`, and writes checksums. `initdata_to_card` constructs and
initializes a temporary `dSv_save_c`, clears its player and horse names, and
copies that save payload into the selected buffer entry. It does not write a
physical card image. Only a later `dataSave()` calls the platform card/NAND save
API.

## Branch A: blank slot

`dFile_select_c::dataSelectStart()` first tests the selected entry's UI-derived
classification. For `mIsNoData[mSelectNum] == false` and
`mIsDataNew[mSelectNum] != 0`, the GCN path performs these route-relevant writes
in order:

1. `mNewFile = 128`;
2. `mDataNum = mSelectNum`;
3. default player name, followed later by confirmed player and horse names; and
4. file-select control advances through name entry until `mIsSelectEnd = true`.

There is no `card_to_memory` call on this branch. The already-live initialized
runtime state remains live. Consequently this branch must not pretend that a
blank physical slot was loaded, and it must not manufacture or mutate a sealed
slot image merely because the user selected its position.

The three blank-slot alternatives differ only in the zero-based `mDataNum`
write and future serialization target. They are mutually exclusive because one
observed selected index is tested per invocation.

## Branch B: existing slot

When the selected entry is neither `mIsNoData` nor `mIsDataNew`,
`dataSelectStart()` writes `mNewFile = 0` and opens the command menu. On GCN the
initial command is Start (`mSelectMenuNum = 1`). `menuSelectStart()` with that
command then performs:

1. `card_to_memory(mSaveData, mSelectNum)`;
2. `mIsSelectEnd = true` and control advances to next-mode wait; and
3. `mDataNum = mSelectNum`.

`card_to_memory` copies exactly the selected `dSv_save_c` projection into live
state, then applies load-time normalization: life is raised to 12 if lower,
dungeon index 6 key count is cleared, hookshot/double-clawshot inventory layout
is normalized, item lineup is rebuilt, vibration is applied, and the displayed
save-stage name is derived from return place. It does not copy `mDataNum`,
`mNewFile`, or `mNoFile` from the card buffer.

For planner execution this is a runtime-lifetime cut:

- the selected sealed slot projection becomes a fresh card-backed runtime;
- session/process state survives outside that lifetime;
- only explicitly named live runtime metadata, notably the header component,
  may be carried and rekeyed without being misattributed to the card image;
- the carried header is then updated with the selected zero-based `mDataNum`;
  and
- initial stage-bank activation and the later world scene request remain
  separate ordered actions.

Delete and Copy (`mSelectMenuNum` 0 and 2) are distinct file-select operations
and are not valid substitutes for Start.

## Branch C: proceed without a usable card/save file

At `MEMCARDCHECKPROC_ERRMSG_WAIT_NO_SAVE_SEL`, accepting the affirmative choice
executes:

1. `setInitSaveData()` for all three session-buffer entries;
2. `card_to_memory(mSaveData, 0)` from newly initialized buffer entry 0;
3. `mNoFile = 1`;
4. `mDataNum = 0`;
5. all three UI entries are classified `mIsDataNew = true`; and
6. player/horse name entry begins.

This creates neither a card-backed runtime nor three physical images. The live
runtime is memory-only, the initialized `mSaveData` entries remain session
buffer state, and `mNoFile` records the no-file path. If a card later becomes
available, `MemCardLoadWait()` can clear nonzero `mNoFile` only after a
successful game-file load.

This branch is mutually exclusive with ordinary blank/existing selection at the
decision point because it is guarded by the no-save prompt control phase rather
than the data-select phase.

## Transition to play scene

Name-scene control observes `mIsSelectEnd`, closes file select, optionally runs
brightness setup, then reaches `changeGameScene()`. That function calls
`dComIfGs_gameStart()`, which requests the current return place. If the selected
entry is classified new, the code overrides the pending next stage with
`F_SP108`, room 1, spawn 21, layer 13. It then requests `PROC_PLAY_SCENE`.

These are three separate facts:

- file selection has ended;
- a next-stage value has been written; and
- the process manager has been asked to create the play scene.

None proves that the world scene loaded. A planner transition must record the
pending destination first and require independent process/load progress before
activating world state, just as the earlier title-to-name-scene request does.

## Required executable-model checks

The implementation should prove all of the following:

- blank, existing, and no-card guards cannot all execute from one observed
  control state;
- selecting blank slot 1, 2, or 3 writes raw data index 0, 1, or 2 without
  populating that physical slot;
- selecting an existing slot restores only its sealed persistent manifest,
  preserves session state, carries only explicitly named runtime metadata, and
  ends the previous runtime lifetime atomically;
- omitted runtime metadata dies at that lifetime cut;
- no-card initialization is represented as a session-buffer-to-live copy, not a
  physical-slot load;
- `mNewFile`, `mNoFile`, and `mDataNum` follow their actual independent writers;
  and
- play-scene requests remain pending until process/world activation is observed.

The save-time `memory_to_card` normalization path, successful physical write,
void/death restart selection, and build/platform variants remain separate audit
and implementation tasks.
