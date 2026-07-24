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
| `dComIfGs_{on,off,is}EventBit` | custom `persistent-event-registers` raw store on `active_runtime_file` | `d_msg_flow.cpp`, `d_save.h`, and audited `saveBitLabels` coordinates |
| query 4 with a nonzero threshold | `inventory-and-resources.rupees` | `d_msg_flow.cpp::query004` and the native player-resource projection |
| event 3 rupee debit | saturating unsigned debit of `inventory-and-resources.rupees` | `d_msg_flow.cpp::event003` queues the negative delta; `d_meter2.cpp::moveRupee` applies it and clamps at zero |
| query 32 health threshold | `inventory-and-resources.life` | `d_msg_flow.cpp::query032` and the native player-resource projection |
| event 5 health debit | saturating unsigned debit of `inventory-and-resources.life` | `d_msg_flow.cpp::event005` queues the negative delta; `d_meter2.cpp::moveLife` applies it and clamps at zero |
| query 22 / event 17 for item `0xa3` | custom `player-light-drop` raw store on `active_runtime_file`, byte `0x04`, mask `0x04` | `d_msg_flow.cpp`, `d_item.cpp`, and `dSv_player_info_c::mLightDrop` |
| `dComIfGs_{on,off,is}SaveSwitch` | `DungeonMemory` on `current_stage`, byte base `0x08`, four-byte big-endian words, 128 switches | `d_msg_flow.cpp` and `dSv_memBit_c::mSwitch` |

The source identities carried by the profile are:

| Source | SHA-256 |
| --- | --- |
| `src/d/d_item.cpp` | `524ce52fb8bb9f6ebbe90dfba67a1eb1e6d7327da5d0c447081818ff902fd1a6` |
| `src/d/d_msg_flow.cpp` | `b50cdeea6508cb997172570fb31531c18def39e84b47e535563c40c93e45f2c1` |
| `src/d/d_meter2.cpp` | `b58ed135700865df0f0cb9ce0e4115de6ec1f9f6dbb8fff8cc1ff99b437d5569` |
| `include/d/d_save.h` | `74a211e5d2ee2c0fe4ce259905fe1f479f373d5b2459d654871cbbd2f61e8756` |
| `src/d/d_save.cpp` | `7e6f09aa36af30932e8ce64423284f885ed0b4e632b22f18d6f0a6b4d104b453` |

## Deliberately unresolved stores

The extracted GZ2E01 resources also use three switch backing classes which the profile
does not bind yet:

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

The planner-owned native boundary can now carry the exact 256-byte
`dSv_event_c::mEvent` payload separately from the label-indexed persistent-flag
diagnostic array. The former is the writable runtime-file backing; the latter
never substitutes for it. Only label indices with audited `saveBitLabels`
coordinates compile to raw reads or writes. Unmapped indices stay explicit
unknowns rather than guessing from their friendly names.

Missing bindings do not erase these nodes. Their encoded control-flow edges are
compiled with explicit `switch-backing` or `branch-predicate` unknown
requirements and no guessed raw read/write. A later actor-entry and zone-context
model can discharge those unknowns without changing the extracted resources.

Rupee query parameter zero is deliberately different: retail compares the
current count with the wallet maximum. Because the native snapshot does not yet
publish that maximum, both outcomes remain explicit `branch-predicate`
unknowns. A nonzero parameter is complete: outcome 0 requires
`rupees >= parameter`, outcome 1 requires `rupees < parameter`, and both
transitions publish a reader over the same structured field.

Health query 32 has no analogous capacity special case: outcome 0 requires
`life >= parameter`, outcome 1 requires `life < parameter`, and both publish a
reader over `inventory-and-resources.life`. Event 5 subtracts its parameter
from that field with a zero floor. Event 4 is deliberately not treated as the
inverse operation: retail healing is capped at four-fifths of maximum life, a
derived runtime ceiling the current state model does not publish.

## Exact extraction smoke test

Against the audited GZ2E01 bundle, the profile constructs all nine selected
programs and the sealed compiler retains every FLW1/FLI1 node. The current base
compile contains 153 query-4 nodes: all 150 nonzero thresholds compile to
complementary exact predicates, while the three parameter-zero nodes remain
unknown. Group 8 node 182 therefore carries the exact 300-rupee threshold
reader used by the cannon-payment graph. All 142 event-3 nodes compile to a
zero-clamped debit, including group 8 node 190's 300-rupee payment. The compile
contains two query-32 nodes, group 1 nodes 553 and 591; both use threshold 4,
so their four outcomes compile to exact complementary life predicates and four
reader records. This exact resource set contains no event-5 node, while a
source-audited compiler regression verifies its zero-clamped life debit. The
compile also retains explicit unknowns for unsupported generic handlers, unmapped
persistent-label coordinates, and 123 event-side `switch-backing` requirements; additional
unresolved switch branch predicates are retained under their branch
requirements. This count is an audit observation, not a schema invariant.

The canonical bundled profile is
`tools/route-planner/crates/engine/data/message-import-profiles/gz2e01-en.json`.
