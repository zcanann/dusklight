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
| `extracted-orig-bundle/v3` | Source manifest; generic actor/scaled/door placements; STAG message group; indexed SCLS destinations; REVT event/exit coordinates; LBNK demo-archive selections; decoded numbered BMG flow graphs; explicit ignored message candidates | KCL/PLC, player-spawn and treasure records, most DZS/DZR chunk bodies, actor parameter meaning, JStudio cutscene internals, message text, runtime bindings |
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
| `extracted-world-facts/v8` | Exact context; source digests; static objects; player spawns; encoded exits; collision-join candidates; exact-GZ2E01 L1-family and L5 boss-door guards, backing-sensitive memory-switch writes, and unresolved physical/actor-phase obligations | Collision triangles, navigation/connectivity, unjoined SCLS activation, other actor-driven exits, item/NPC/event producers, other door families, live actor reconstruction |
| `orig-bundle-diff/v1` | Exact input-bundle/content digests; raw-versus-decoded stage/message/ignored-candidate differences; explicit one-sided locale coverage | Semantic comparison for domains not decoded into the input bundles |
| `fact-pack/v1` plus immutable cache | Exact content, extractor, source, coverage, payload, and manifest identities; verified reuse without `orig/` | Any implication that a partial coverage declaration is complete |

The planner owns its copies of the `WorldContext` and `WorldInventory` wire
contracts in `world_data.rs`. The compatible producer currently lives elsewhere
in the repository, but the planner has no Rust dependency on Huntctl and must
not acquire one. `extracted-orig-bundle/v3` is planner-native. There is not yet a
transform from that bundle into `WorldInventory` or directly into a complete
planner fact pack.

## Exact GZ2E01 extraction coverage

The retail acceptance bundle contains:

- 3,661 sealed input files;
- 384 decoded stage/room archives;
- 6,128 DZS/DZR chunk directory entries;
- 2,102 entries whose record layout is currently recognized and 4,026 whose
  record body is not decoded;
- STAG data in 79 archives;
- 1,036 indexed SCLS destinations;
- 29,184 generic actor/scaled/door placement records;
- BMG flow groups 0 through 8 for locale bundle `us`; and
- one explicit empty candidate, `Msgus/bmgres99.arc`.

The counts are coverage evidence, not proof that every represented record has
gameplay semantics. In particular, an SCLS destination is inert until an
activation contract is known, and a placement's raw parameters do not prove its
guard, switch ownership, lifecycle, or interaction geometry.

The most consequential current parity mismatch is visible in the source data:
`PLYR` appears in 304 archives and `TRES` in 128, but both remain undecoded in
the planner-native orig extractor even though the compatible world-inventory
schema has typed player-spawn and treasure records. Other frequent undecoded
families include `RARO`, `RCAM`, `FILI`, `LBNK`, `LGT*`, `REVT`, `RPAT`,
`RPPN`, `RTBL`, and environment/color records. Their chunk tag, count, and
offset remain visible; their record bodies and meanings do not.

## What the current world import really does

`ExtractedWorldFacts::build` verifies exact content/runtime/world-context
digests, then imports:

- placements as static world objects;
- `PlayerSpawn` placements as spawn records;
- every SCLS record as an encoded destination; and
- each precomputed collision/SCLS join as an encoded-map-exit candidate.

Every collision-join candidate still receives a geometry obligation. Joins
marked `inferred_semantics` also receive an unknown activation-semantics
requirement. Unjoined SCLS records remain encoded facts and do not become
executable transitions.

The importer does **not** import the inventory's KCL triangles or spatial BVH.
It retains the spatial-index digest and names abstract source/destination region
IDs, but feasibility must come from separately modeled spatial observations,
refinements, or future planner-owned geometry facts. It also ignores treasure
semantics and does not decode any actor-specific placement parameters.

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
- Decode `PLYR` and `TRES` in the native extractor and establish parity tests
  against compatible world inventories.
- Import actor-driven map changes, doors, portals, elevators, warps, event
  transitions, cutscene scene changes, restart/savewarp, void, death, and title
  transitions as distinct classes.
- Decode the guard, consumption, persistence, and reconstruction behavior for
  keyed doors and other route-critical actor families.
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
