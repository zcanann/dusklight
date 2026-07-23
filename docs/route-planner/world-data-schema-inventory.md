# Route-planner world-data schema inventory

Status: audited against the planner contracts at `824315df62` and the canonical
GZ2E01 extraction with bundle SHA-256
`c3408c1ccbf0c4596b5989b0cc94c2c1db9680d38b4336dd7bf425f07de6a78c`.
This inventory describes what each artifact actually proves, not everything its
name might suggest.

## Artifact boundaries

| Artifact | Current represented facts | Deliberately absent or still missing |
|---|---|---|
| `orig-input-scan/v1` | Disc-header product/platform/region/revision, normalized `sys/` and `files/` manifest, executable/game-data/resource digests, candidate archive paths | Friendly identity, runtime language selection, decoded records, behavior |
| `supported-build-registry/v1` | Exact full-fingerprint-to-friendly-ID mapping | Nearest-build fallback, implied cross-build equivalence |
| `extracted-orig-bundle/v4` | Source manifest; separate generic actor/scaled/door, treasure (`TRES`/`TRE*`), and player-spawn (`PLYR`) placements with raw records; STAG message group; indexed SCLS destinations; REVT event/exit coordinates; LBNK demo-archive selections; decoded numbered BMG flow graphs; explicit ignored message candidates | KCL/PLC, most other DZS/DZR chunk bodies, actor/treasure parameter meaning, JStudio cutscene internals, message text, runtime bindings |
| `cutscene-wrapper-topology/v1` | Exact joins among one REVT event, layer LBNK demo archive, `event_list.dat` staff/cut/data paths, map-tool ID, and normal/skip SCLS records | JStudio phase semantics, exceptional resource-failure dispatch, return/restart writers, executable transition effects |
| `binary-function-evidence/v1` | Exact DOL/symbol-table identities, one bounded text symbol, DOL section/address/file coordinates, selected code bytes and digest, and exact `blr`-only immediate-return classification | Call-site reachability, function-name semantics, larger-function disassembly, source control flow, or cross-build equivalence |
| `binary-range-evidence/v1` | Exact DOL identity, one bounded virtual-address range wholly contained in exactly one loadable text/data section, section/address/file coordinates, and selected bytes/digest | Semantic meaning, proof that code references the bytes, pointer/control-flow analysis, BSS, or cross-build equivalence |
| `jstudio-stb-program/v1` | Exact archive/resource identities; bounded STB/FVB/object/command/paragraph structure; object and function IDs; waits, suspends, jumps, and reserved controls; hashed payloads | Executable-specific object/adaptor meaning, gameplay state effects, and compiled cutscene transitions |
| `jstudio-adaptor-profile/v1` and `jstudio-semantic-program/v1` | Exact content/executable binding; audited selector dispatch; typed variable/adaptor payloads; explicit unresolved records | Proof that a request executed, actor lifecycle/results, message completion, gameplay writes, and scene changes |
| `demo-actor-program/v1` | Exact GZ2E01 binding of `d_actN` to `Demo00`; decoded status-51 packed raw commands; persistent/temporary event-bit effects; exact reserved-raw coverage outside generic actors | Proof that an authored actor was created or executed, ordering against non-JStudio writers, or equivalence on another build |
| `cutscene-package-runtime-profile/v1` and `resolved-cutscene-package/v1` | Exact wrapper/nominal-STB join; archive failure behavior; ordered STB fallbacks; missing-parse and mode-zero PACKAGE control flow; nominal actor ID-write summaries | Producer proof for a corruption setup, final outer-event exit, other return-place writers, or an implicit savewarp |
| `cutscene-outer-runtime-profile/v1` and `resolved-cutscene-outer-event/v1` | Exact package, raw stage/event-list resources, PLAY-to-WAIT local completion flags, REVT skip-cut fields, and exact-context normal/skip/suppressed candidate transitions | The actor-corruption producer, witnessed corruption suffix or actual branch, another return-place writer, or an implicit savewarp |
| `cutscene-corruption-hypothesis/v1` | Event-generic exact-context unknown-evidence producer whose sole effect is the named all-STB-lookups-missing field, with explicit failure-site, predicate, and completed-prefix unknowns | Exact-build evidence that actor corruption produces that failure, runtime dispatch flags, location, return-place writes, or savewarp |
| GZ2E01 tower return-place mechanics | Atomic three-field `Savmem` write; raw M_012/M_014 and `NO_TELOP` backing predicates; ordinary savewarp readers and a dynamic location load | Proof that the tower actor executes after the witnessed allocation failure, the incoming Castle Town return-place bytes, or the exceptional event suffix |
| `message-flow-entry-contract-set/v4` and `compiled-message-flow-entry-set/v4` | Exact compiled-flow schema/digest, stage/message resource joins, optional raw actor placement identity, resolved flow label, authored guards, obligations, unknowns, speaker context, source-pinned presentation requests, resolved generic item backings, and deterministic entry mechanics | Unauthored callers, presentation-actor execution, inferred interaction geometry, actor behavior not established by placement, or equivalence across builds/languages |
| `dusklight-world-context/v1` | One game-data digest and sorted stage-to-inventory/spatial-index digests | Product/revision identity, runtime configuration, the inventory or spatial records themselves |
| `dusklight-world-inventory/v1` | Source records; chunk directory; actor/scaled/door/treasure/player placements; SCLS; KCL prisms and decoded PLC words; inferred same-room collision/SCLS joins | Actor-specific guards and lifecycle, dynamic collision, paths/rails/volumes, source-confirmed activation behavior |
| `extracted-world-facts/v16` | Exact context; source digests; static objects; player spawns; encoded exits; collision-join candidates; per-candidate reconstructed trigger triangles, planes, bounds, and same-room spawn references; exact-GZ2E01 L1/L5 boss doors, keyed mini-boss doors, key shutters, Lakebed boss shutter, memory-switch-backed type-0 Koki gates, the two SCLS-backed L7 bridge-demo branches, F_SP109 and F_SP121 rider gates, the R_SP116 wolf-chain writer and externally switched shutter, and both F_SP118 caravan-gate parents with distinct transient key and boar-destruction branches; backing-sensitive guards/effects; source-and-placement-bound L1/L5 oriented/compound interaction shapes; mechanics-v29 staged obligations | Collision navigation/connectivity, proof that a same-room spawn reaches a trigger, unjoined SCLS activation, other source-censused but semantically unaudited actor-driven exits, item/NPC/event producers, unavailable native attention-position observations, and actor families without an audited reconstruction rule |
| `orig-bundle-diff/v2` | Exact input-bundle/content digests; raw-versus-decoded stage/message/ignored-candidate differences; explicit one-sided locale coverage; a sealed domain matrix that marks executable code, runtime language selection, actor semantics, cutscene semantics, and rule semantics `not_represented` | Comparison data for domains explicitly reported as unrepresented |
| `fact-pack/v1` plus immutable cache | Exact content, extractor, source, coverage, payload, and manifest identities; verified reuse without `orig/` | Any implication that a partial coverage declaration is complete |

The planner owns its copies of the `WorldContext` and `WorldInventory` wire
contracts in `world_data.rs`. The compatible producer currently lives elsewhere
in the repository, but the planner has no Rust dependency on Huntctl and must
not acquire one. `extracted-orig-bundle/v4` is planner-native. There is not yet a
transform from that bundle into `WorldInventory` or directly into a complete
planner fact pack.

## Exact GZ2E01 extraction coverage

The retail acceptance bundle contains:

- 3,661 sealed input files;
- 384 decoded stage/room archives;
- 6,128 DZS/DZR chunk entries with their tag, count, and offset retained; v4
  additionally decodes every `PLYR`, `TRES`, and layered `TRE*` record instead
  of leaving those chunk bodies opaque;
- STAG data in 79 archives;
- 1,036 indexed SCLS destinations;
- 29,184 generic actor/scaled/door placement records;
- BMG flow groups 0 through 8 for locale bundle `us`; and
- one explicit empty candidate, `Msgus/bmgres99.arc`.

The counts are coverage evidence, not proof that every represented record has
gameplay semantics. In particular, an SCLS destination is inert until an
activation contract is known, and a placement's raw parameters do not prove its
guard, switch ownership, lifecycle, or interaction geometry.

V4 closes the former placement-layout parity mismatch: `PLYR` appears in 304
archives and `TRES` in 128, and the planner-native extractor now emits their
same 32-byte name/parameter/position/angle/set-ID records into separate
`player_spawns` and `treasure_placements` collections. Layered `TRE*` chunks
retain their decoded layer. Frequent still-undecoded families include `RARO`,
`RCAM`, `FILI`, `LGT*`, `RPAT`, `RPPN`, `RTBL`, and environment/color records.
Their chunk tag, count, and offset remain visible; their record bodies and
meanings do not.

The exact R_SP116 room-6 resource
`10487ef6754fec1f454c93aa33f605ee9781b4db4b91eed8e864721d76304d40`
is the retail parity witness: both the independent compatible inventory and
planner-native v4 extraction produce 95 actor placements, five player spawns,
zero treasures, and one 32-byte `PLYR` chunk with five records. The engine test
reproduces this comparison whenever the original tree is present.

## What the current world import really does

`ExtractedWorldFacts::build` verifies exact content/runtime/world-context
digests, then imports:

- placements as static world objects;
- `PlayerSpawn` placements as spawn records;
- every SCLS record as an encoded destination; and
- each precomputed collision/SCLS join as an encoded-map-exit candidate.

For exact GZ2E01 L1/L5 boss doors, v16 also imports the actor-local `checkArea`
rectangle (`|x| <= 200`, `|z| <= 100`) using the placement's binary-angle yaw,
and the shortest-circular-facing obligation around `door_yaw - 0x7fff`. L5
adds the strict positive-local-Z `checkFront` plane. L1 adds form-selected
human/wolf compound checks, including the wolf attention point and independent
current-position local-X strip. These observations are bound to both the world
inventory and audited actor source. They do not discharge the separate loaded-
actor/event-phase obligation.

For the exact D_MN07 room-6 `dr` placement, v16 separately imports the two
source branches. Outside layer 3, a positive small-key count and the exact
world-X/Z start box lead to SCLS 6 without consuming a key or writing the
bridge switch. On layer 3, actor creation requires switch `0x18` clear, but the
event itself has no key guard; its confirmed suffix writes switch `0x18` and
selects SCLS 7. Event acceptance, camera/player ownership, and bridge animation
completion remain staged actor/interaction obligations.

For every exact F_SP109 room-0 `R_Gate` layer, v16 keeps three causal states
separate: a locked key event can decrement one key and set memory switch
`0x6b`; an already-set switch permits physical pushing; and persistent event
bit M_035 (`saveBitLabels[68]`, raw coordinate `0x0810`) forces both leaves
fully open without consuming a key or writing that switch. V16 applies the same
four mutually exclusive branches to the exact F_SP121 room-3 and room-15
copies, binding switches `0x82` and `0x81` to the complete 64-byte
dungeon-session Boolean view instead of misclassifying them as stage-memory
switches.

For the exact R_SP116 room-6 pair, v16 imports the one-shot `Wchain` producer and
state-neutral `vshuter` consumer separately. The chain's `0x00000fef`
parameters normalize repeat nibble `0xf` to false; a wolf pull past the exact
94-unit switch offset writes current-room one-zone switch `0xef`. The shutter
has no internal key guard or writer and becomes passable only after that shared
state is set. Wolf attention/jump/tension/pull execution and the shutter's
opening/collision phases remain explicit staged obligations. The exact world
inventory is SHA-256
`44d58b8a8b8b4b8ba5f24bf192c61092a9176075d0e2409a797905406c384601`.

For the exact F_SP118 room-1 and room-2 `CrvGate` parents, v16 imports normal
and high-count key-event branches whose only persistent effect is the eventual
one-key decrement. The parent-created child leaf, paired transient `SetOpen`,
camera/event completion, and absence of any unlock switch remain one staged
effect obligation. The ridden-boar collision/destruction path is a separate
state-neutral bypass with its event, actor, speed, distance, collision, and
paired-destruction conditions retained as an activation obligation.

Every collision-join candidate still receives a geometry obligation. Joins
marked `inferred_semantics` also receive an unknown activation-semantics
requirement. Unjoined SCLS records remain encoded facts and do not become
executable transitions.

The importer does **not** import the inventory's KCL triangles or spatial BVH.
It retains the spatial-index digest and names abstract source/destination region
IDs, but feasibility must come from separately modeled spatial observations,
refinements, or future planner-owned geometry facts. It also ignores treasure
semantics. Actor-specific parameter decoding is limited to the named,
exact-GZ2E01 families above; every other placement remains opaque.

## Missing-field backlog

### Identity and selection

- Audit and represent how runtime language/configuration selects `Msg*`
  resources. A locale directory is observed input, not proof that it is active.
- Catalogue remaining retail fingerprints and language bundles. Exact GZ2E01 US
  is the only bundled identity today.
- Bind every generated world artifact to the exact content and extractor/schema
  identity used to create it; do not rely on stage names alone.

### Topology and activation

- Connect planner-native orig records to base facts without routing through
  Huntctl implementation types.
- Connect the planner-native placement collections directly to base world facts
  without routing through compatible world-inventory artifacts.
- Import actor-driven map changes, doors, portals, elevators, warps, event
  transitions, cutscene scene changes, restart/savewarp, void, death, and title
  transitions as distinct classes.
- Continue importing other source-censused actor writers and consumers against
  their exact state domains. V16 already composes the R_SP116 `Wchain` producer
  with its `vshuter` consumer and binds F_SP121 rider actions to the observed
  dungeon-session view. The exact GZ2E01 keyed-family census is recorded in
  `docs/route-planner/gz2e01-keyed-door-gate-family-audit.md`.
- Preserve source-room/layer activation rules separately from destination
  fields. Layer placement alone is not an activation predicate.

### Geometry and live state

- Add planner-owned KCL/PLC and room-transform facts, or a strictly versioned
  import for the existing wire artifacts.
- Model dynamic/moving collision, water and trigger volumes, paths/rails,
  clearance, one-sided surfaces, room connectivity, and containment.
- Separate static placement, persisted control state, and live actor instances;
  import reconstruction/despawn rules rather than assuming a placed actor is
  loaded or blocking.
- Turn approach geometry into state-evaluated obligations without declaring a
  collision/SCLS join physically reachable merely because both records exist.

### Messages, events, and cutscenes

- Decode message text/index resources and all relevant query/event-handler
  semantics, not only the currently recognized flag/switch accesses.
- Import item handoffs, pending item actors, event names, normal/abnormal
  cleanup, interaction/attention volumes, player-control gates, and temporal
  windows.
- Import concrete event/cutscene phase data into the existing phase-program IR,
  including scene changes, return-place writers, resource requests, failure
  branches, and exact affected-field masks.

### Storage and save semantics

- Decode actor parameters and source operations that read/write event bits,
  temporary bits, stage memory, dungeon keys/items, return places, and global
  session stores.
- Bind local stage memory to its physical backing and import normal commit/load,
  save/load/title, void/death, and runtime-file lifecycle operations.
- Retain unknown record bodies as unavailable coverage. Future derived decoders
  must be rerun from user-supplied originals; fact packs must not embed original
  copyrighted chunk payloads merely to make later decoding convenient.

## Schema-change acceptance rule

A world-data task is complete only when all of the following are true:

1. The source bytes and exact content context are identified.
2. The decoded field has a bounded, strict, versioned contract.
3. Raw coordinates needed to reproduce the decode are retained where lawful.
4. Unknown, malformed, absent, and inferred values remain distinguishable.
5. The importer demonstrates whether the field is an inert fact, a hard guard,
   a state operation, a physical obligation, or evidence only.
6. A retail or synthetic acceptance case proves the intended behavior and a
   negative case proves that missing data cannot become a permissive default.
7. Coverage manifests and build/language diffs include the new domain.

This rule prevents a larger schema from silently becoming a more permissive
route graph.

## Authoritative code and evidence

- `tools/route-planner/crates/engine/src/orig_discovery.rs`
- `tools/route-planner/crates/engine/src/orig_extraction.rs`
- `tools/route-planner/crates/engine/src/orig_diff.rs`
- `tools/route-planner/crates/engine/src/world_data.rs`
- `tools/route-planner/crates/engine/src/world_import.rs`
- `docs/route-planner/orig-discovery-and-extraction.md`
- `docs/glitch-hunting/world-inventory.md` and `world-spatial.md` for the
  compatible producer's current wire-level coverage and stated limits
