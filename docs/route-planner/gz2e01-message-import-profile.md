# GZ2E01 English message import profile

This profile is limited to the exact bundled GameCube USA 1.0 `GZ2E01`
content digest and runtime language `en`. It selects the nine `Msgus` resources
(`bmgres.arc` and groups 1–8) found by the exact orig extraction. Product ID or
the `us` directory name alone is not accepted as equivalence evidence.

## Audited bindings

The profile publishes only backing relationships supported by the current
source and state representation. A snapshot or authored start state must supply
the corresponding exact raw component; label-indexed observation arrays never
stand in for it.

| Message operation | Planner backing | Evidence |
| --- | --- | --- |
| `dComIfGs_{on,off,is}TmpBit` | `TemporaryFlags` on `active_runtime_file` | `d_msg_flow.cpp`, `d_save.h`, and the exact temporary-label table |
| `dComIfGs_{on,off,is}SaveSwitch` | `DungeonMemory` on `current_stage`, byte base `0x08`, four-byte big-endian words, 128 switches | `d_msg_flow.cpp` and `dSv_memBit_c::mSwitch` |

The source identities carried by the profile are:

| Source | SHA-256 |
| --- | --- |
| `src/d/d_msg_flow.cpp` | `b50cdeea6508cb997172570fb31531c18def39e84b47e535563c40c93e45f2c1` |
| `include/d/d_save.h` | `74a211e5d2ee2c0fe4ce259905fe1f479f373d5b2459d654871cbbd2f61e8756` |
| `src/d/d_save.cpp` | `7e6f09aa36af30932e8ce64423284f885ed0b4e632b22f18d6f0a6b4d104b453` |

## Deliberately unresolved switch stores

The extracted GZ2E01 resources also use four backing classes which the profile
does not bind yet:

- persistent event-register bytes addressed through `saveBitLabels`;
- 146 accesses to `dSv_danBit_c` dungeon-session switches;
- 57 accesses to zone switches; and
- 162 accesses to one-zone switches.

`dSv_danBit_c` is a live bank with its own stage number and reset/init behavior;
it is not the persisted current-stage `dSv_memBit_c` bank. Its label-indexed
native observation therefore has a custom diagnostic component kind. Treating
both as one `DungeonMemory` component would hide wrong-flags and transfer
questions.

Zone and one-zone handlers resolve a zone from the message speaker's room, or
from the current stay room when there is no speaker. A room-to-zone lookup and
the actual speaker context are therefore required. Binding these handlers to
`current_room` would be wrong for actors outside the player's room and for
multiple rooms sharing a zone.

The native observation currently exposes label-indexed persistent flag results,
not the unique raw event-register array. Those observations are useful facts but
cannot safely receive a packed-coordinate write. Persistent generic handlers
therefore remain unknown until that raw backing is captured.

Missing bindings do not erase these nodes. Their encoded control-flow edges are
compiled with explicit `switch-backing` or `branch-predicate` unknown
requirements and no guessed raw read/write. A later actor-entry and zone-context
model can discharge those unknowns without changing the extracted resources.

## Exact extraction smoke test

Against the audited GZ2E01 bundle, the profile constructs all nine selected
programs and the sealed compiler retains every FLW1/FLI1 node. The current base
compile contains explicit unknowns for unsupported generic handlers, persistent
flag backing, and 123 event-side `switch-backing` requirements; additional
unresolved switch branch predicates are retained under their branch
requirements. This count is an audit observation, not a schema invariant.

The canonical bundled profile is
`tools/route-planner/crates/engine/data/message-import-profiles/gz2e01-en.json`.
