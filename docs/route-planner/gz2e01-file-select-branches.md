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
function-range extraction seals the promoted load-normalization and name-entry
details to the recognized retail DOL. The name-entry artifacts are:

| Function | VA | Size | Code SHA-256 | Artifact SHA-256 |
| --- | ---: | ---: | --- | --- |
| `nameInput__14dFile_select_cFv` | `0x801873bc` | `0x13c` | `0388366b478b3a51aa2a7cd4c7825eb7370dec67b14e3b7db98e2c9aad284ba5` | `fd93ea0a72e1008434af10c19cd8f59a430f01bd8a044f5173bd97e78bd6ae0a` |
| `nameInputFade__14dFile_select_cFv` | `0x8018759c` | `0x104` | `1972401d18a34e1f1d8c6ab180df465df2c17d34a9fc03dbcdda37b1229249d8` | `ecb601568e64364a3adfc779bf737949371a1460c1daca3651ec31ef1631c726` |
| `nameInput2Move__14dFile_select_cFv` | `0x801876a0` | `0xac` | `a96931c928651f29eea71bf214964abe46f8af5a7a3006581153fef732c614e5` | `9da639084fa4d342c1154c2669aa65eb22c81d3fa52b9281f0ab100c15a86f33` |
| `nameInput2__14dFile_select_cFv` | `0x8018774c` | `0xd8` | `32fb5e79113d0a52bde235fd8c1fb3c052b66445bc1b7264e8c065d53e5ea87b` | `e7a2a4b3ed67e42938aa0a28f2deaa66edab757618d0bcacdaef3598e627cc13` |

All four artifacts record executable SHA-256
`e7f197436815e66c4a11df3d7bd557d66083b641ff8a8e76439f3caba7ae60e8`
and symbol-table SHA-256
`8b8c98b86b6270543709adbbd489ca4a5cd4fa5c30fd4a410420702fd37a085a`.

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

## Name-entry completion

Player information is a runtime-file-owned `dSv_save_c` component. The planner
keeps each meaningful NUL-terminated name as bytes, not UTF-8 text: the retail
setters copy game-encoded data and the model must not reinterpret it. Bytes
after the terminator in each 17-byte destination array are unprojected because
`strcpy` does not establish them. The exact GZ2E01/English initializer supplies
the `Link` and `Epona` byte strings. `initdata_to_card` instead writes an empty
string to each name in every transient file-select buffer; the no-card branch
restores that blank projection and then writes both localized defaults before
input opens.

The successful new-file suffix is now an ordered executable chain:

1. player confirmation copies the observed submitted byte string into live
   player info and enters the 15-tick fade;
2. fade completion restores the default horse name and starts the second fade;
3. horse input becomes available only after its timer reaches zero while reset
   is clear; and
4. horse confirmation copies the observed bytes and writes
   `mIsSelectEnd = true`, represented by `selection_end`.

Horse Back executes both 15-tick return phases before player input becomes
active again; the confirmed player bytes survive. Player Back is split by
`mNoFile`: an ordinary blank-slot path moves toward data selection, while a
no-card path returns to memory-card status checking. Input results, timers, and
reset state are observed transient control fields. Missing observations leave
the edge unknown instead of silently finishing the UI.

None of these operations writes a physical slot. In particular,
`dFile_select_c::nameInput2()` sets `mIsSelectEnd` but never calls `dataSave()`.
The later memory-to-card projection and platform write are the separate path
described below.

## Successful physical save: promoted neutral branch

The ordinary in-game save menu uses a distinct asynchronous pipeline from
file-select name confirmation. Five exact GZ2E01 functions seal the promoted
path:

| Function | VA | Size | Code SHA-256 | Artifact SHA-256 |
| --- | ---: | ---: | --- | --- |
| `putSave__10dSv_info_cFi` | `0x800350f0` | `0x5c` | `f94364f83aed527671a218a8e0a5b2a9e541578fbd775176981f22df31fddd6e` | `eb3032a28f0a4d08684d74894785c1760a241020d907b12bee19e350eda1caf9` |
| `memory_to_card__10dSv_info_cFPci` | `0x80035798` | `0x26c` | `7cf6fc958ed1e4cdcf4b3e168364cbd7a42a545a1812d139a4442e41ae5fd8e9` | `5b65a8833c8fb246e5c0292e0f22ecf6b05f5e3a123f2f18ee33c343a9805f1e` |
| `dataWrite__12dMenu_save_cFv` | `0x801f2840` | `0xa4` | `b6a30e6925392a2c876f0f002e93afeb257da6878b989515c12fe83b58c6ac35` | `cf1308d2ecb1741549ce173a76f7e7c0ff8fe7343156632baae499dea1836ebb` |
| `memCardDataSaveWait__12dMenu_save_cFv` | `0x801f28e4` | `0xa8` | `ab833e5d0f988b09921e3788272ebaa325767f91f649af3209ff0bcff6b40778` | `8b8f2e635426fdd8dc3e4cf4c49953ef1518e6836dca669acbd5cd5706ad0394` |
| `memCardDataSaveWait2__12dMenu_save_cFv` | `0x801f298c` | `0x1d0` | `206affd3eccd29c55beed5853501307985d355504ab3c4d5ebbb076dd719022f` | `c0bdf0610b4b25b22ddf5dab9745bbf8dfdd8267d02daaf878186335eb3b1d88` |

`dataWrite()` commits the current live stage memory into its saved stage bank,
projects live `dSv_save_c` into the selected zero-based entry of the already
loaded three-entry buffer, recalculates that entry's checksum, and submits the
whole game file. Writing the whole buffer does not semantically replace the
other two slots: their loaded entries are unchanged.

Submission is not success. `memCardDataSaveWait()` polls until the command
finishes, and `memCardDataSaveWait2()` distinguishes result `1` from result `2`
after the wait timer expires. Only result `1` writes `mDataNum` to the selected
index, clears `mNoFile`, refreshes the displayed save data, and enters either
the continue prompt (`mUseType` 1/2) or event save-end path (`mUseType` 3/4).
Result `2` enters the error UI and must not create a sealed planner slot or
claim either header write.

Mechanics catalog v26 promotes that successful completion for the exact neutral
projection currently expressible by native components. The guards require:

- active world execution and an observed loaded save buffer;
- `data_save_wait2`, command result `1`, timer zero, one selected index, and an
  exact use type;
- either the monkey-lantern recovery event bit set or both transient stolen /
  dropped bits clear; and
- all projected first-item acquisition bytes zero, which proves that the
  conditional missing-lantern inventory/oil rewrite cannot run.

The active-runtime save operation derives its persistent image identity at
execution time, so it remains valid after arbitrary prior loads. It seals the
named runtime-file projection plus every available stage bank, overwrites only
the selected physical slot, and leaves the active runtime lifetime intact. A
separate failure transition proves that result `2` changes no slot.

This is deliberately not yet the universal `memory_to_card` transform. When
the lantern is acquired but absent from slot 1, the serialized image temporarily
receives the lantern and backed-up oil; when recovery is incomplete, stolen and
dropped event bits are temporarily cleared. Total-time/date fields also remain
outside the current player-info projection. Those transformed branches stay
open rather than inheriting the neutral result.

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

Mechanics catalog v26 and execution state v15 now prove the backing-store and
branching subset below:

- blank, existing, and no-card guards cannot all execute from one observed
  control state;
- selecting blank slot 1, 2, or 3 writes raw data index 0, 1, or 2 without
  populating that physical slot;
- selecting an existing slot restores only its sealed persistent manifest,
  preserves session state, carries only explicitly named runtime metadata, and
  ends the previous runtime lifetime atomically;
- omitted runtime metadata dies at that lifetime cut;
- no-card initialization is represented as a session-buffer-to-live copy, not a
  physical-slot load, and all three initialized buffer entries remain explicit,
  independently inspectable custom stores after entry 0 is copied;
- `mNewFile`, `mNoFile`, and `mDataNum` follow their actual independent writers;
- submitted name bytes enter the runtime-file save projection only on their
  respective confirmation edges, horse Back preserves the player name, and
  neither confirmation creates or changes a physical slot; and
- a generic active-runtime load derives both its fresh lifetime ID and the exact
  selected sealed manifest at execution time rather than embedding ephemeral
  runtime/persistent IDs in authored mechanics.

Exact DOL artifacts seal `card_to_memory__10dSv_info_cFPci` at `0x80035a04`
(460 bytes, artifact `fca390c6...dc12352`) and
`setLineUpItem__17dSv_player_item_cFv` at `0x800332f8` (92 bytes, artifact
`f9edd7f1...bf7a14`) against the recognized executable and symbol-table
identities. Existing-slot Start now executes the conditional 12-life minimum
through a generic typed unsigned clamp: lower values rise to 12 and higher
values retain their value and last-writer provenance.

The same exact-context transition now clears dungeon-6 key count, applies both
hookshot slot rewrites, rebuilds the 24-byte lineup in the audited 23-slot
priority order, copies saved vibration into live session state, and derives the
displayed save-stage from the restored return place. The item operation is
generic and parameterized by slot/item values and lineup order; it does not
embed GZ2E01 constants in the executor. Exact-DOL-sealed confirmation/fade
transitions and source-audited Back transitions now produce `selection_end`
from observed input bytes and timer/reset state. Separate new-file and
existing-file transitions then record the requested `PROC_PLAY_SCENE` and
pending destination while keeping `PROC_NAME_SCENE` active and the retained
last world non-traversable.

The existing-slot transition is executable for this audited post-copy prefix:
its digest-verified sealed image copy, lifetime cut, and all route-relevant
`card_to_memory` normalization effects are modeled in order. Blank/no-card
selection and both pending play-scene request shapes remain independently
executable.

The non-neutral `memory_to_card` lantern/event projection, unprojected time/date
fields, void/death restart selection, and build/platform variants remain
separate audit and implementation tasks.
