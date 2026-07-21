# Text Displacement message-state audit

Status: exact GZ2E01 extraction and source-level generic-handler audit. Producer
microtraces and the complete Goron Mines consumer/effect chain remain separate
11J tasks.

## What is actually shared

`dMsgFlow_c` does not own a private dialogue-progress integer. Generic message
flows read and write `dSv_info_c::info.getTmp().event` through
`dComIfGs_{on,off,is}TmpBit`. This process/runtime temporary-event backing store
is shared by otherwise unrelated NPC flow instances.

The ten general-purpose message-flow coordinates cleared by the central event
cleanup are:

| Label-table index | Source symbol | Packed byte/bit coordinate | Friendly planner name |
|---:|---|---:|---|
| 11 | `T_0010` | `0x0004` | `message_flow_control_a` |
| 12 | `T_0011` | `0x0002` | `message_flow_control_b` |
| 13 | `T_0012` | `0x0001` | `message_flow_control_c` |
| 14 | `T_0013` | `0x0180` | `message_flow_control_d` |
| 15 | `T_0014` | `0x0140` | `message_flow_control_e` |
| 51 | `T_0051` | `0x0508` | `message_flow_control_f` |
| 52 | `T_0052` | `0x0504` | `message_flow_control_g` |
| 53 | `T_0053` | `0x0502` | `message_flow_control_h` |
| 54 | `T_0054` | `0x0501` | `message_flow_control_i` |
| 55 | `T_0055` | `0x0680` | `message_flow_control_j` |

These are packed coordinates, not ten consecutive bits and not an actor-local
field. Label index 10 is `T_0009`, an Ordon tutorial flag, and must not be
mistaken for flow-control A.

## Generic access semantics

The relevant source handlers in `src/d/d_msg_flow.cpp` are exact:

- `event010` decodes two big-endian `u16` label indices and sets each nonzero
  referenced temporary bit.
- `event011` decodes the same shape and clears each nonzero referenced bit.
- `query011` reads one label index and returns branch outcome 1 when the bit is
  clear, or outcome 0 when it is set.

An on-disc branch node does **not** store the numbered handler. It stores an
index into `dMsgFlow_c::mQueryList`. Raw index 10 dispatches to `query011`.
The first eight dispatch entries are reordered as 5, 1, 2, 3, 6, 7, 4, 8;
raw indices 8 through 52 dispatch to handlers 9 through 53. The extractor emits
both `raw_query_index` and `query_handler_index` so a route rule cannot silently
confuse these identities.

## Cleanup paths

`clear_tmpflag_for_message` in `src/d/d_event.cpp` clears exactly label indices
11–15 and 51–55. It runs when event status 5 is completing or changing scenes
and either event flag2 bit 2 is set or the skip timer is negative. This is a
conditional cleanup edge, not a universal property of every room load, void, or
ordinary NPC interaction.

`daObjWarpOBrg_c` in `src/d/actor/d_a_obj_warp_obrg.cpp` independently clears
the same ten indices during its Ooccoo warp behavior. The planner must retain
these as two separately evidenced operations. Other title/load/reset boundaries
still require boundary-specific observation rather than inheriting a guessed
"all text flags clear" rule.

## Retail extraction result

The new planner-owned commands perform bounded Yaz0 decoding, unique-basename
RARC extraction, BMG FLW1/FLI1 parsing, and DZS/DZR actor-placement parsing.
They have no Huntctl or TAS dependency.

For the supplied GZ2E01 US message group 3 input:

- archive `orig/GZ2E01/files/res/Msgus/bmgres3.arc`:
  `4f61ed3a4a603d6c6d00801e4e41b10ff7d7d787dc9848ec880ba275556bb0fb`
- resource `zel_03.bmg`:
  `7fa2a522b4f65eafd0a9e31cbe2226abfed852e232aa25d20e817845969c5b8b`
- 454 flow nodes, 50 labels, and 42 generic temporary-flag accesses were
  decoded.

Across message groups 0–8 the same extraction produced 14,020 nodes and 859
temporary-bit accesses. Of those, 634 reference flow-control A–J. Counts are
inventory evidence only; reachability from a particular NPC's flow label must
be established through graph edges and the stage-selected message group.

The placement audit also establishes the Gor Coron input chain without guessing
the map or flow ID:

- `R_SP110/STG_00.arc` selects message group 3 through STAG byte `mMsgGroup`.
- `R_SP110/R00_00.arc` places `grD1` on layers 1, 2, and 3 with
  `home.angle.x == 6`; `daNpc_Grd_c::create` copies that value into `mFlowID`.
- Therefore the relevant consumer starts at flow label 6 in `zel_03.bmg`, node
  56. The exact displaced predicates and later access-control effects are left
  open until their full flow/cutscene/actor chain is audited; this document does
  not promote the community-observed outcome into a direct named write.

Exact placement inputs:

- `R_SP110/R00_00.arc`:
  `eaa765317343f775676ca19a53819ec188a79598e0b974caaa361cc2eed26067`
- extracted `room.dzr`:
  `887ce68064f9f26713497f70734c3ab65b3bf82162665087e3eafab8f43a5109`
- `R_SP110/STG_00.arc`:
  `cc235f8ed662a096989eed6c605838bd3ae0836155db0778602611effbfad60b`
- extracted `stage.dzs`:
  `89f22211de029bb4ecbc0ea01915da144a0d13d0e9613d012e815b31f7bddb4b`

## Current modeling boundary

This milestone proves backing identity, raw coordinates, generic producers,
generic consumer polarity, conditional cleanup, deterministic extraction, stage
message-group selection, and Gor Coron's authored flow-label input. It does not
yet prove a Coro/Auru/Yeta/Ooccoo interruption witness, Gor Coron's downstream
event/switch writes, or the independent wall/elevator/NPC/reload obligations.
Those remain explicit 11J work rather than being collapsed into a
`text_displacement = true` shortcut.
