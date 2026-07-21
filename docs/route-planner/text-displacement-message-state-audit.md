# Text Displacement message-state audit

Status: exact GZ2E01 extraction, source-level handler audit, and complete Gor
Coron consumer/entrance acceptance model. Physical producer witnesses retain
their separately documented version and community evidence boundary.

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

The same extractor now emits the access semantics needed downstream:

- `event000`/`event001` set and clear persistent `saveBitLabels` entries;
- `query001` takes its true branch when the referenced persistent bit is clear;
- `event014`/`event015` set and clear save, dungeon, zone, or one-zone switches;
- `query013`, `query015`, `query017`, and `query019` take their true branch when
  the corresponding switch is clear; and
- event nodes preserve the full big-endian 32-bit parameter in addition to its
  two `u16` views, which is required for exact flow-jump auditing.

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
- 454 flow nodes and 50 labels were decoded. Temporary, persistent, and switch
  accesses are emitted as separate typed collections.

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
  56.

Exact placement inputs:

- `R_SP110/R00_00.arc`:
  `eaa765317343f775676ca19a53819ec188a79598e0b974caaa361cc2eed26067`
- extracted `room.dzr`:
  `887ce68064f9f26713497f70734c3ab65b3bf82162665087e3eafab8f43a5109`
- `R_SP110/STG_00.arc`:
  `cc235f8ed662a096989eed6c605838bd3ae0836155db0778602611effbfad60b`
- extracted `stage.dzs`:
  `89f22211de029bb4ecbc0ea01915da144a0d13d0e9613d012e815b31f7bddb4b`

## Exact Gor Coron consumer

Flow 6 begins at node 56 and first checks persistent label 64, `M_031` (Goron
Mines clear). When that bit is clear, the displaced branch reaches node 31 and
tests temporary label 13, flow-control C. A set C jumps through event009 at node
48 to flow 9. If C is clear, node 34 tests label 12, flow-control B; a set B
passes through node 49/message 126, node 42 sets C, and node 48 makes the same
flow-9 jump.

Flow 9 begins at node 205 and tests temporary label 11, flow-control A. If A is
clear, its longer dialogue path ends at node 206 by setting A, so another talk
is required. With A already set, the path reaches node 190. Node 190 executes
`event000` with persistent label 62 and therefore sets packed coordinate
`0x0704`, `M_029` (won the Gor Coron match). Nodes 189 and 208 then clear A, B,
and C. The exact displaced-win predicate is consequently:

```text
M_031 is clear
AND flow-control A is set
AND (flow-control B is set OR flow-control C is set)
```

This is represented as a derived fact over raw backing stores. It is not a
manually asserted `text_displacement` capability. Producer transitions write B
or C, the intermediate Gor Coron interaction writes A, and the consumer reads
the derived predicate, sets M029, and clears A/B/C.

## Downstream access and physical blockers

The entrance is encoded independently of story authorization. `R_SP110` room
data contains SCLS exit 0 to `D_MN04` and a `scnChg` actor using that exit. The
acceptance catalog therefore exposes `transition.r-sp110-scls0-goron-mines`
before the route is executable, then auto-binds three obstructions to that exact
transition and approach:

1. `daObjGraWall_c::Create` rejects the parameter-`0xff` wall when M029 is set,
   and `Execute` deletes an already-live wall as soon as M029 becomes set.
2. Type-4 `daNpc_grA_c` gate actors initialize/move from M029. The mode-1 actor
   controls the elevator approach through switch `0x6f`; its teach-elevator
   event sets the switch and begins the gate movement.
3. Already-live Goron actors do not all reconstruct at the instant M029 changes.
   A room reload rebuilds their state from M029, while the witnessed route can
   instead roll past the residual live blocker after the wall disappears.

The wall deletion, elevator authorization/switch write, NPC reload
reconstruction, and roll-past bypass are separate resolvers. No resolver is
encoded as an unconditional side effect of M029, and reconstruction rules are
independently replayable from persisted M029.

## Solver acceptance

`solver::tests::goron_text_displacement_composes_raw_consumer_and_independent_entrance_blockers`
proves the joined graph:

- the raw encoded exit remains visible but reports all three active blockers
  when M029 is clear;
- each enabled Coro, Auru, Yeta, or Ooccoo producer can independently feed the
  unchanged Gor Coron consumer and entrance chain;
- backward relevance from `D_MN04` contains all four producers;
- removing the sole enabled producer makes the route unreachable;
- adding a hypothetical producer makes it reachable only under research
  evidence policy, without editing the consumer or SCLS transition; and
- room-reload and roll-past NPC handling remain distinct successful routes.

The acceptance fixture intentionally proves causal composition and backing-store
identity. Physical Coro/Auru/Yeta/Ooccoo timing and geometry are version-scoped
as described in `text-displacement-producer-model.md`; production fact packs
must not generalize those witnesses beyond their evidence.
