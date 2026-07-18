# Serious glitch-hunting framework backlog

This is the implementation backlog for turning Dusklight into a serious,
deterministic Twilight Princess glitch-reproduction and glitch-discovery
platform. It intentionally covers more than reinforcement learning. DDQN, CQL,
or any future learner still needs the same trustworthy simulator, observations,
actions, corpus, objectives, and replay proof.

The checked-in route tree remains structurally simple: segments have segments
as children, and alternative attempts are siblings. Goals, model scores,
novelty, and proof are metadata attached to a segment or run. Search results do
not become route history until explicitly promoted.

## How to use this file

- `[x]` means the foundation exists, not necessarily that it has reached its
  final performance or fidelity target.
- `[ ]` is implementable work. Each workstream lists an acceptance gate so a
  partially implemented feature cannot be mistaken for a reliable capability.
- **P0** work blocks trustworthy search. **P1** makes the framework broadly
  useful. **P2** is scale, specialized fidelity, or research work.
- Every serialized schema, query, model, scenario, tape, trace, and proof must
  carry enough identity to reject incompatible reuse.
- `docs/glitch-hunting/` explains the current architecture. This file owns the
  cross-cutting work queue and should link to detailed designs as they emerge.

## Non-negotiable invariants

- Normal playback and discovery are read-only with respect to gameplay state.
  They may control the exclusive emulated PAD, logical pacing, presentation,
  automation-owned artifacts, and process lifecycle. Game observations enter
  automation through `const` access.
- Observation support must not modify decompiled gameplay logic, data flow, or
  object behavior merely to make private state convenient to inspect. Query
  implementations live out of line under the fork-only automation boundary,
  are conspicuously compile-time gated, and are absent from native/upstream
  builds that do not opt into that boundary.
- Native gameplay code is not an observation implementation surface. Do not add
  query getters, query virtuals, instrumentation members, instrumentation state,
  wrapper branches, or alternative code paths to gameplay classes. A native
  header may expose only the smallest layout-neutral, default-off aperture
  required by an out-of-line reader: a guarded forward declaration plus a
  guarded `friend` declaration. Prefer no aperture at all when public `const`
  state is sufficient.
- Decompiled/gameplay translation units are not query implementation sites. If
  no stable outer sampling boundary exists, the only permitted intrusion is a
  minimal, unmistakably `#if DUSK_ENABLE_AUTOMATION_OBSERVERS`-guarded call to
  a fork-owned read adapter; the surrounding native statements and control
  flow must remain unchanged when the block is preprocessed away. Runtime
  `if`, a generic PC-port pragma, or an `IF_DUSK` branch is not an observation
  boundary.
- Observation adapters may copy already-realized state only. They must not call
  mutating or non-`const` gameplay helpers, trigger lazy initialization, fill a
  cache, allocate from a game heap, advance RNG, issue a fresh collision query,
  or otherwise cause game-visible work. Prefer public `const` access. If a fact
  is truly inaccessible, a narrowly scoped compile-gated friend/read adapter is
  the last resort and requires an explicit field-by-field audit plus A/B replay
  parity evidence; changing gameplay code or layout is not an acceptable
  shortcut.
- `const` is necessary but not sufficient. Each invoked native helper must have
  an audited, side-effect-free implementation; otherwise the adapter reads the
  underlying field directly through its bounded friend aperture. Never assume a
  method is safe merely because its signature is `const`, or because mutation is
  not obvious at its call site.
- Query code is instrumentation, never native gameplay implementation. Every
  query-only include, friend declaration, adapter body, and sampling hook must
  be visibly delimited by `DUSK_ENABLE_AUTOMATION_OBSERVERS`; compiling with the
  option off must erase it. Even an observationally equivalent gameplay edit is
  forbidden when an out-of-line read adapter can obtain the same fact.
- Use the dedicated observer macro as the pragma-like boundary. Observation
  apertures in native files must be explicitly labeled as Dusklight
  observation-only code and enclosed by
  `#if DUSK_ENABLE_AUTOMATION_OBSERVERS`; generic `TARGET_PC`, runtime flags, and
  naming conventions do not qualify. With the gate off, native object layout,
  virtual tables, initialization, control flow, and gameplay-visible work must
  be identical to the upstream path.
- Experimental writes are a separate compile-time-disabled intervention
  capability with explicit runtime opt-in and an unavoidable mutation audit.
  An intervention result is never represented as ordinary TAS proof.
- Logical simulation ticks, never wall time, determine input, reward, events,
  objectives, replay, and score.
- A learned or reactive policy is only a proposal generator. Promotion requires
  a realized absolute input tape, exact predicate proof, and cold deterministic
  replay with no policy in the loop.
- Headful, hidden-headful, and headless execution must preserve game-visible
  simulation. Rendering work may only be removed after parity evidence proves
  it irrelevant.
- An identical build, scenario, and absolute tape must produce an identical
  logical result on every run. Any disagreement is a framework determinism bug
  requiring first-divergence investigation; it is never a reason to mine a
  supposedly more robust tape, add timing slack, or weaken the proof.
- A faster local segment does not automatically dominate a slower one with a
  different RNG state, actor state, loader state, or downstream opportunity.
- Native pointers and process-local IDs may be diagnostics, but they are not
  portable identity.
- Missing observations stay explicitly missing. They must never silently
  become zero, nearest-actor fallback, or a stale value.
- The unit of evidence is an episode or intervention, not a collection of
  correlated frames presented as independent samples.

## Existing foundation

- [x] Exact four-port `DUSKTAPE` codec, exclusive input ownership, recording,
  deterministic tape/controller handoff, and realized-tape output.
- [x] Compact TAS and controller DSLs, including timed layers, stick curves,
  coordinate seek, actor seek, button overlays, and additive composition.
- [x] Fixed-step logical clock, unpaced execution, hidden windowed fast-forward,
  null-renderer execution, and retained-frame live handoff.
- [x] Versioned native bootstrap worker and Rust worker pool supervision.
- [x] Direct `--stage STAGE,ROOM,POINT,LAYER` process launch, optionally
  combined with `--load-save SLOT`, plus first-class process/stage boot origins
  in `DUSKTAPE` v3.
- [x] Content-addressed corpus and route artifacts with SHA-256 verification.
- [x] Authored read-only predicates, milestone evidence, boundary fingerprints,
  and segment-attached goals/proofs.
- [x] Route segment graph, human child recording, playback, thumbnails, draft
  promotion, and Git-backed topology editing.
- [x] Bounded actor catalog and exact placed-actor selectors.
- [x] Immutable gameplay trace v1 decoding plus the initial Trace v2
  channel-directory writer/decoder, transition extraction, deterministic tree
  FQI, behavior archive, structured search, Q proposal evaluation, and cold
  replay.
- [x] Safe Eye Shredder neighboring-memory model and name-entry instrumentation.

These are foundations rather than completion claims. Known gaps include
asynchronous timing, complete RNG coverage, engine-session worker commands,
portable reset/checkpoints, canonical state hashes, collision/contact
observations, general actor state, and a sufficiently Markov trace.

## Dependency order

```text
determinism + identity
        |
        +--> persistent scenario/reset/checkpoint execution
        |
        +--> typed world/query toolbox --> trace v2 --> counterfactual corpus
                                      |                   |
                                      +--> goals/oracles  +--> FQI/CQL/IQL/DDQN
                                      |                   |
                                      +--> option actions +--> planning/search
                                                          |
                                      proof + cold replay <-+
                                                          |
                                      route graph / Skybook benchmarks
```

The query toolbox, option actions, and corpus are not throwaway RL scaffolding.
They remain useful for manual TAS work, exhaustive search, causal experiments,
debugging, and future algorithms.

## 1. Run identity, determinism, and fidelity

### P0: complete run identity

- [ ] Include the Dusklight commit and dirty-tree digest, Aurora commit,
  compiler, configuration, feature switches, target architecture, protocol
  capabilities, and fidelity profile in every run artifact.
- [ ] Hash the DVD/game-data inputs, region, language assets, scenario fixture,
  predicate program, action schema, observation schema, and relevant settings.
- [x] Define compatibility rules separately for replay, trace merging, model
  training, checkpoint restore, and cross-build comparison.
- [ ] Make every CLI and workbench action reject mismatched identity instead of
  implicitly using whatever binary, disc, or configured path is available.
- [ ] Add a human-readable identity diff explaining every rejection.

### P0: first-divergence tooling

- [ ] Define canonical state-hash tiers: core game state, route boundary,
  extended gameplay state, and full supported checkpoint state.
- [ ] Hash normalized values rather than padding, pointers, allocator addresses,
  or unordered container traversal.
- [ ] Store periodic hashes and allow dense hashes around a suspected desync.
- [ ] Build a replay comparator that binary-searches to the first divergent tick
  and prints a typed field/event diff.
- [ ] Compare realtime headful, unpaced headful, hidden-headful fast-forward,
  and null-renderer runs over a shared conformance corpus.
- [ ] Automatically quarantine a tape/scenario/build combination after any
  repeated-run disagreement and retain all attempts for divergence analysis.
- [ ] Classify and trace the nondeterministic source—input, logical time, RNG,
  asynchronous completion, uninitialized state, process leakage, floating
  point, rendering traversal, or observation side effect—before search resumes.
- [ ] Reject “stability rate,” extra neutral frames, repeated button presses,
  reactive waits, or a different candidate as substitutes for fixing identical
  absolute-tape replay.

### P0: deterministic time and asynchronous systems

- [ ] Inventory every game-visible clock: SDK time, alarms, host time calls,
  audio clocks, movie clocks, loader timeouts, and third-party library clocks.
- [ ] Drive `OSAlarm`, `__OSGetSystemTime`, and all simulation-visible timers
  from the logical tick model.
- [ ] Make shader compilation and nondeterministic host I/O pause simulation
  without consuming PAD reads, RNG draws, events, or score ticks.
- [ ] Capture and control resource-loader completion ordering where it affects
  gameplay.
- [ ] Audit audio, movie, job, and streaming threads for game-visible ordering.
- [ ] Record unavoidable external completion events as explicit replay inputs
  until they can be made deterministic.

### P0: RNG coverage

- [ ] Inventory all random streams: global `cM`, actor-local `cM_rnd_c`, JMath,
  Z2, particle, manager-owned, and subsystem-specific generators.
- [ ] Add versioned, read-only per-tick snapshots or call counters for relevant
  streams without advancing them.
- [ ] Attribute RNG draws to stream and, where practical, call site or subsystem.
- [ ] Include objective-relevant RNG in boundary/archive identity without
  pretending it is the complete process state.
- [ ] Add tests proving capture is allocation-free and observationally inert.

### P1: fidelity matrix

- [ ] Build a machine-readable matrix for native safety fixes, `AVOID_UB`, GC
  layout emulation, relative/absolute MEM1 address behavior, floating-point
  behavior, GX traversal, cache behavior, and console-only quirks.
- [ ] Let each benchmark declare required fidelity capabilities.
- [ ] Classify failures as unsupported fidelity rather than ordinary goal misses.
- [ ] Maintain a small emulator/console transfer suite for effects the native
  port cannot faithfully render or execute.

**Acceptance:** the same conformance tapes agree tick-for-tick across supported
execution modes, or terminate with a typed, localized capability/divergence
report. A same-mode repeated-run disagreement blocks the affected framework
configuration until explained and fixed. No search result is ranked when
determinism evidence is contradictory.

## 2. Persistent execution, reset, and checkpoint acceleration

### P0: engine-session worker

- [ ] Extend the persistent worker beyond `hello`/`ping`/`shutdown` to own a
  loaded engine session.
- [ ] Implement versioned binary or shared-file commands for scenario load,
  tape/program upload, batch run, reset, trace configuration, screenshot,
  checkpoint, and shutdown.
- [ ] Upload controller data once per batch; never send one IPC request per tick.
- [ ] Return structured terminal reasons, predicate results, events, hashes,
  performance counters, crashes, and artifact references.
- [ ] Add watchdog heartbeat, hard tick/memory/output bounds, crash isolation,
  and automatic worker replacement.

### P0: scenario fixtures

The checked-in process-boot tapes are not yet robust or broad enough to serve
as the validation harness for every map-local capability in this backlog.
Direct stage boot is therefore a first-class test origin: it should make it
cheap to author a short tape for an arbitrary map and goal—walk toward a
target, exercise an actor, cross a trigger, test collision, or inspect a
transition—without first extending a fragile title/menu/route prefix.

- [x] Give authored tapes/programs an explicit, versioned boot origin:
  `process` for normal executable boot or `stage` for a targeted fixture start.
  Do not silently change the meaning of existing raw `DUSKTAPE` files; use a
  versioned tape launch envelope/manifest if the input codec itself should
  remain pure controller data.
- [x] Define the initial `stage` boot descriptor as stage ID, room, spawn/point,
  and layer, with optional save-slot or named loadout/fixture identity. Map the
  first implementation onto Dusklight's existing `--stage` and `--load-save`
  launch options rather than inventing a second stage loader.
- [x] Bind boot origin and every stage/loadout field into scenario identity,
  boundary fingerprints, corpus entries, traces, results, and proofs. Reject a
  tape when its declared boot origin was not actually established.
- [x] Start tape tick zero only after the declared stage fixture reaches its
  explicit readiness predicate. Loading, shader work, and host-only waits must
  not consume tape input or become timing slack in the authored program.
- [x] Let `huntctl` compile, run, record, minimize, and inspect stage-boot tapes
  directly so a test author can pair any targeted map start with an arbitrary
  predicate/goal and promote the realized tape like any other test artifact.
- [x] Define canonical, versioned fixtures for save data, stage, room, layer, entrance, form,
  inventory, equipment, flags, health, RNG, video mode, and settings.
- [x] Separate readiness from replay: fixture loading may wait before tick zero;
  tape playback may only assert readiness and fail.
- [x] Build a stage/room smoke catalog with expected initial fingerprints.
- [ ] Detect durable leakage across runs: save data, memory card, globals,
  managers, caches, particles, audio, loader queues, and temporary files.
- [x] Keep clean-boot and fixture-start leaderboards distinct. Population,
  results, evaluation plans/reports, and leaderboard rows carry one exact boot
  origin; population construction and ranking reject mixed origins.

### P1: checkpoint tiers

- [ ] Keep exact prefix replay as the portable baseline and report its cost.
- [ ] Add explicitly serialized game-state checkpoints one subsystem at a time,
  including pointer fixups and reconstruction rules.
- [ ] Investigate Windows process snapshots or a platform forkserver only after
  the engine-session path is stable.
- [ ] Bind each checkpoint to build, scenario, parent tape, tick, full supported
  state hash, thread/resource state, and a validation window.
- [ ] Quarantine a checkpoint permanently after any validation mismatch.
- [ ] Support `play from parent`, `run suffix from parent`, and `replay visually`
  through the fastest validated tier while retaining the absolute tape.

### P1: rendering and throughput

- [ ] Measure cost by simulation, GX traversal, backend rendering, shader work,
  resource I/O, tracing, hashing, IPC, and process reset.
- [ ] Remove the hidden SDL/taskbar surface from true headless operation if the
  backend can drain GX without it.
- [ ] Preserve a presentation-capable hidden-headful path for instant live
  handoff and screenshots.
- [ ] Precompile/warm shader variants with visible host-only progress while
  simulation remains frozen.
- [ ] Add per-capability throughput benchmarks in deterministic candidate-ticks
  per second, not rendered FPS.

**Acceptance:** one warm worker evaluates many suffix candidates without process
restart or state leakage; every accelerated checkpoint reproduces a validation
window and the final candidate still passes a clean-process absolute replay.

## 3. The read-only game query toolbox

The toolbox must be broader than the current model features. We eventually want
to ask questions nobody anticipated when the trace schema was designed.

Use three layers:

1. **World inventory:** complete, relatively static map and placement data,
   captured or indexed once per game-data identity.
2. **Live query service:** bounded native queries over current game state,
   actors, contacts, geometry, flags, UI, and subsystems.
3. **Observation view:** a compact objective-specific tensor/table compiled
   from queries for a controller, search job, or model.

Every rock, tree, bush, actor placement, trigger, path, and collision polygon
should eventually be discoverable through the first two layers. They should not
all be copied into every per-frame neural observation.

### P0: schema and query foundations

- [x] Migrate the legacy milestone, reactive-controller, and actor-catalog
  reads out of `m_Do_main.cpp` and into the compile-gated observer boundary.
  The legacy non-`const` `getRunEventName()` observation is gone; milestone
  result and boundary-fingerprint v2 encode its absence explicitly.
- [ ] Move the remaining name-entry and file-select private-state capture out of
  their gameplay translation units. Prefer a narrow compile-gated friend/read
  adapter implemented in `dusk/automation`; leave at most a side-effect-free
  sampling call at the native phase boundary. Do not inject convenience query
  methods into gameplay classes.
- [x] Put the general milestone, controller, catalog, and Trace v2 native query
  adapters in the fork-owned `dusk/automation` boundary behind the single
  default-off `DUSK_ENABLE_AUTOMATION_OBSERVERS` compile-time gate.
- [x] Add a build/CI check that fails when observer code, friend declarations,
  or query-only includes leak into an ungated upstream/native configuration.
- [ ] Maintain an access manifest for every surfaced field: declaring type,
  read expression, phase, portability, access mechanism, and side-effect audit.
- [ ] Require an exception record before adding any friend/read adapter for
  private state, including why an out-of-line public/const observation cannot
  provide the fact. Never alter object layout, virtual dispatch, initialization,
  or gameplay control flow for observation.
- [ ] Standardize and statically enforce an explicit observation-only aperture
  marker around every native forward/friend declaration. Reject inline adapter
  bodies, query convenience methods, query state, and any aperture not erased by
  `DUSK_ENABLE_AUTOMATION_OBSERVERS=OFF`.
- [ ] Add observer-off/on ABI conformance checks for every native class touched
  by a friend aperture: equal `sizeof`, `alignof`, relevant `offsetof` values,
  vtable shape, and construction/destruction behavior. A friend declaration
  should be the only permitted difference and must remain layout-neutral.
- [ ] Define a stable typed fact schema with field IDs, scalar/vector/enum/bitset
  types, units, coordinate spaces, missingness, sampling phase, and version.
- [ ] Define audited observation phases at meaningful engine boundaries such as
  pre-input, pre-actor execution, post-movement/pre-collision, post-collision,
  post-event, and post-simulation. Only expose phases backed by a stable native
  insertion point.
- [ ] Separate portable facts from diagnostic facts such as addresses.
- [ ] Add a native read-only adapter registry for player, actors, managers,
  collision, event, save, UI, loader, and map data.
- [ ] Audit every adapter for hidden mutation, lazy initialization, game-heap
  allocation, cache updates, or collision helpers with side effects.
- [ ] Run observer-on versus observer-off A/B conformance over identical tapes;
  require identical canonical state hashes, RNG snapshots/counters, events,
  terminal state, and replay proof. Treat any difference as a framework bug.
- [x] Split write-capable original-console fidelity models from read-only
  observation behind the separate default-off
  `DUSK_ENABLE_AUTOMATION_FIDELITY_MODELS` gate. Search builds force it off;
  runtime opt-in remains mandatory when it is compiled in.
- [ ] Return immutable snapshots; never expose a live pointer over IPC.
- [ ] Support bounded selection, filtering, sorting, nearest-K, aggregation,
  spatial predicates, and explicit truncation metadata.
- [x] Implement the offline static-geometry slice of that contract: mandatory
  room scope, pre-ranking exact trigger/destination filters, stable nearest-K,
  AABB broad-phase and finite double-sided rays, 1..=256 result bounds, explicit
  truncation, and node/triangle accounting. This is not the live query service.
- [ ] Compile declarative query specifications ahead of a run. Do not parse a
  general query language or allocate dynamically in the per-tick hot path.
- [ ] Hash the exact query/observation specification into traces and models.
- [x] Provide offline re-featurization so new model views can be produced from
  sufficiently rich raw traces without rerunning the game. The first compiled
  view is the objective-authenticated `movement-state/v2` Ordon exit view.
- [x] Define `movement-state/v2` around Trace v2 presence/status semantics so
  it never maps an unavailable legacy field (currently event-name hash) to
  zero or merges it under the legacy movement-v1 schema digest. Semantic
  absence is represented by authenticated mask fields; unavailable and
  truncated required channels are typed extraction failures.

### P0: player observation

- [ ] Position, previous position, velocity, acceleration, forward speed,
  facing, shape angle, movement angle, and camera-relative angle.
- [ ] Actor/profile identity, form, procedure, subprocedure, mode flags, and
  animation identity/frame/rate.
- [ ] Action timers and phases for roll, jump, attack, item, damage, ledge,
  crawl, climb, swim, Epona, wolf, and cutscene-controlled states.
- [ ] Grounded/falling/swimming/climbing/crawling/riding, invulnerability,
  knockback, target/lock-on, held/carried actor, and item-use state.
- [ ] Health, magic, air, stamina-like meters, inventory/equipment, item slots,
  and contextual action prompt.
- [ ] Collision body/extents and the exact correction/displacement applied this
  tick.
- [ ] Normalized PAD consumed by the game plus a short configurable input
  history window.

### P0: actor and object observation

- [ ] Enumerate all live process groups with stable runtime generation handles.
- [ ] Preserve placed identity using game-data digest, stage, room, actor type,
  map set ID, parameters, and home transform.
- [ ] Surface parent/child/owner/target/mount/carry relationships.
- [ ] Surface transform, velocity, facing, scale, collision extents, health,
  procedure/state, animation, timers, status, attention flags, and spawn/death
  phase through typed common fields.
- [ ] Allow actor-type-specific read-only adapters for important internal facts
  without bloating every actor record.
- [ ] Query by exact placed identity, stable runtime handle, symbolic type,
  parameter mask, relationship, room, state, tag, and spatial relation.
- [ ] Emit spawn, delete, damage, state-change, target-change, carried, mounted,
  and contact events.
- [ ] Replace the current fixed lowest-process-ID sampling behavior with explicit
  deterministic query budgets and truncation semantics.

### P0: static world inventory

- [ ] Parse or capture stage/room metadata, actor placements, paths, rails,
  spawn points, exits, doors, triggers, switches, cameras, event placements,
  collision meshes, water, void/death planes, and special surfaces.
- [x] Add a bounded offline RARC/DZS/DZR/KCL/PLC inventory slice for F_SP103:
  recognized placements, player spawns, SCLS exits, every addressable collision
  prism, and collision-exit-to-SCLS trigger joins. Unknown chunks remain
  enumerated; this does not claim paths, rails, regions, or events are decoded.
- [x] Assign stable IDs to the implemented static records based on source
  content digests and structural record location rather than runtime address.
- [x] Preserve authored KCL source indices, heights, PLC material/code words,
  reconstructed polygons/planes, and explicit degeneracy in a
  content-addressed map artifact. Retail degeneracy is retained, never skipped.
- [ ] Build spatial indices for nearest polygon, region containment, ray/sweep,
  route/load trigger, ledge, clearance, and local neighborhood queries.
- [x] Build the first content-addressed per-room spatial slice: canonical
  median-AABB BVHs over every reconstructed KCL triangle, retained degeneracy
  exclusions, nearest point queries, AABB neighborhood candidates, finite rays,
  and exact load-trigger/destination filtering. Region, sweep, ledge, and
  clearance semantics remain open.
- [ ] Generate semantic tags where the game data provides them; keep inferred
  tags explicitly marked as inferred.
- [ ] Add an inspector that can answer “what is this object/polygon/trigger?”
  from a world coordinate and show the source record.
- [x] Add the collision-surface portion of that inspector through `huntctl world
  query point|aabb|ray`, returning stable source identity, raw PLC facts,
  geometry, distances/intersections, and optional SCLS trigger metadata.

### P0: collision and local geometry

- [x] Replace Trace v2's provisional nearest-`SCENE_EXIT` actor-origin distance
  before it is used by `movement-state/v2`. Decode the realized oriented exit
  volume and destination, report inside/signed-distance/latch/commit state, and
  retain the ingredients for stable placed identity; an actor origin kilometers
  from its realized volume is not geometry. Scene-exit v2 and its old v1 decoder
  are distinct wire contracts; movement-state/v1 rejects v2 rather than silently
  reinterpreting it. This channel describes `SCENE_EXIT`/`SCENE_EXIT2` actors,
  not every transition mechanism.
- [x] Add the first read-only Link background-collision slice by copying the
  already-resolved Acch ground/roof/water/wall caches, polygon/owner presence,
  stored ground plane, and old-to-final frame displacement. This is not yet
  actor/attack/push contact coverage or per-pass correction attribution.
- [x] Resolve transition metadata from Link's already-cached collision polygons,
  including exit ID, room SCLS destination, material/code, and stable polygon
  identity. The checked `F_SP103` to `F_SP104` route is ground-polygon-driven;
  its unrelated live `SCENE_EXIT` actor remains outside and points back into
  `F_SP103`, so actor-volume telemetry must never stand in for this load zone.
  Optional Trace v2 channel 10 copies all six bounded Acch surfaces, preserves
  raw DZB/KCL codes and geometry indices, and joins prism 2217 to the
  content-addressed room-1 KCL/PLC source without calling gameplay queries.
- [ ] Join collision exit polygons to static triangle/region geometry so a
  controller can optimize signed distance and approach direction before the
  transition fires, without issuing a fresh gameplay collision query. The
  offline inventory and per-room BVH now join and spatially query every
  reconstructable F_SP103 KCL/PLC exit surface against same-room SCLS metadata.
  The remaining work is compiling bounded static features into a task-local
  controller/model observation rather than querying the game.
- [ ] Surface ground, wall, ceiling, water, actor, attack, and push contacts with
  subject IDs, polygon IDs, normals, penetration, relative velocity, material,
  and begin/persist/end phase.
- [ ] Record pre-correction position, proposed displacement, collision response,
  and final position for Link and selected actors.
- [ ] Identify which collision/correction pass produced each displacement so
  within-tick ordering bugs are visible rather than collapsed into one final
  position.
- [ ] Provide bounded radial raycasts and shape casts at configurable heights.
- [ ] Provide local height, slope, clearance, ledge/void distance, wall angle,
  and load-zone plane features.
- [ ] Build optional small local occupancy, height-field, or signed-distance
  patches for models; identify their coordinate frame and resolution exactly.
- [ ] Detect collision crossing, tunneling, NaN, excessive displacement, and
  contradictory contact flags as events/oracles.

### P0: stage, event, transition, and save state

- [ ] Current stage/room/layer/point, requested next stage, transition phase,
  fade/loading state, spawn resolution, and relevant trigger identity.
- [ ] Event identity, name, phase, staff/cutscene ownership, dialogue/message
  state, camera ownership, and control-lock reason.
- [ ] Room switches, event bits, temporary flags, item flags, dungeon state,
  boss flags, save slots, death/revival, warp/Ooccoo, and memory-card state.
- [ ] Provide named typed flag registries where known, while retaining raw
  versioned bitsets for future reinterpretation.
- [ ] Emit flag, event, dialogue, cutscene, transition-request, load-start, and
  load-complete deltas.

### P1: UI and menu state

- [ ] Typed screen stack and modal ownership for title, file select, name entry,
  pause, map, item wheel, collection, save, continue, and game-over flows.
- [ ] Cursor/index, enabled options, transition animation, accepted/rejected
  input, and menu-local timers.
- [ ] Text-entry layout, backing bytes, modeled original offsets, and writes.
- [ ] UI events suitable for exact boot/menu tape minimization without reactive
  timing in the final proof.

### P1: resource, heap, and process state

- [ ] Resource/archive requests, ownership, queue state, completion events, and
  game-visible load dependencies.
- [ ] Actor/process counts by group/profile, slot exhaustion, create/delete
  queues, and failed allocations.
- [ ] Heap identities, capacity/high-water/failure events, allocation classes,
  and GC-relative offsets where meaningful.
- [ ] Bounded watchpoints for selected fields, guards, invariants, and original
  layout models without making ordinary observation a gameplay write.
- [ ] Allocation/free and process-create/delete events with heap, size,
  alignment, caller, owning actor/process, result, and logical phase.
- [ ] Thread/current-heap and DVD/ARAM/resource-transfer events for benchmarks
  whose failure mechanism depends on scheduling or a null destination.
- [ ] Crash, assertion, hang, deadlock suspicion, invalid handle, and memory
  corruption artifacts with last-known events and inputs.

### P1: camera, rendering, audio, and effects

- [ ] Camera transform, target, mode, owner, lock-on, clipping, and transition
  state where it affects control or a glitch predicate.
- [ ] Render/effect state needed for visual glitches, while keeping it out of
  movement models unless explicitly requested.
- [ ] Audio manager/music state for sound-manager, queueing, and silence bugs.
- [ ] Particle/effect RNG and actor linkage where effects influence gameplay or
  fidelity diagnosis.

### P1: toolbox UX and APIs

- [ ] `huntctl inspect scene`, `inspect actor`, `inspect collision`, `inspect
  flags`, `inspect rng`, and `query` commands against a paused/live worker or
  artifact.
- [ ] Schema discovery with names, units, enum values, availability, cost, and
  fidelity requirements.
- [ ] Symbolic research-field registry that may bind a build-specific native
  field or class offset, while recording that binding and never presenting it
  as portable identity.
- [ ] Live read-only overlay for Link state, selected actors, contacts, rays,
  triggers, paths, flags, timers, and objective values.
- [ ] Click or spatial-pick an actor/polygon in a graphical run and copy a stable
  selector/query into a controller, predicate, or experiment.
- [ ] Save named query presets in Git and reference them from goals, trace
  profiles, model views, and benchmarks.
- [ ] Provide a bounded remote inspection API; never make the browser capable of
  arbitrary memory access or filesystem paths.

**Acceptance:** given an arbitrary Skybook setup, an author can inventory the
room, identify exact actors and collision, inspect relevant flags/timers/RNG,
and compile a bounded observation view without changing gameplay. Full map
geometry is stored once; normal movement traces contain only requested local or
semantic features plus stable references back to the inventory.

## 4. Action and tactic toolbox

### P0: exact controller action space

- [x] Preserve the full PAD surface: all buttons, main/sub sticks, triggers,
  analog A/B, connection/error state, and all four ports.
- [x] Support lossless splice, trim, layer, resample-for-authoring, diff, and
  delta minimization while retaining canonical 30 Hz output.
- [x] Represent button edge, hold, release, and illegal/conflicting combinations
  explicitly in the authoring layer.
- [x] Define state-dependent action masks as search guidance, never as proof
  restrictions that could hide a glitch-producing “invalid” input.

### P0: temporally extended options

- [x] Define a versioned semi-Markov option schema: type, parameters, duration,
  cancellation/termination condition, emitted raw actions, and realized tape
  range.
- [x] Add move in world/player/camera coordinates toward a coordinate, plane,
  path point, placed actor, runtime actor, or inferred opening.
- [x] Add turn, brake, neutral, align, maintain heading, and maintain distance.
- [x] Add roll with direction, button frame, recovery, cancellation, and
  deterministic roll-spacing phase.
- [x] Add jump attack, normal attack/combo, shield, target, interact, item use,
  transform, crawl, climb, swim, Epona, boomerang, clawshot, spinner, and other
  typed game-specific tactics as benchmarks require them.
- [x] Add waypoint, rail, spline, and Bézier movement with exact duration and
  sample phase.
- [x] Allow bounded observation feedback such as seek actor or maintain offset;
  record every realized raw frame for proof.
- [x] Provide option-relative local golf that adjusts heading, magnitude,
  duration, phase, button timing, and cancellation around a successful result.

### P0: composition semantics

- [x] Define deterministic priority and composition for base motion, additive
  correction, button overlays, camera/sub-stick layers, and safety clamps.
- [x] Reject ambiguous overlapping writers rather than depending on file order.
- [x] Bound every loop, wait, branch, feedback controller, and target-loss path.
- [x] Make loss of an exact target a typed terminal/option result; never switch
  silently to nearest.
- [x] Compile static programs to canonical tapes where possible and label
  reactive executions with the exact observation provenance used.

### P1: tactic diagnostics

- [x] Record option start/end/reason, intended target, error vector, action mask,
  raw output, clamps, and game-consumed input.
- [x] Visualize option intervals, curves, targets, contacts, and goal progress on
  the route graph and gameplay overlay.
- [x] Maintain reusable tactic tests per player procedure and game mode.

**Acceptance:** a search policy can choose a compact option and parameter tuple,
the native worker executes it without per-tick IPC, and the result materializes
as an exact raw tape. The same toolbox supports human-authored TAS work and
algorithmic exploration.

## 5. Goals, predicates, reward, and oracles

### P0: extensible predicate DSL

- [x] Compile predicates over typed facts, events, stable actor selectors,
  geometry relations, flags, timers, hashes, and bounded temporal conditions.
- [x] Support exact equality, ranges, regions, planes, contact relationships,
  state transitions, persistence for N ticks, and ordered event sequences.
- [x] Add named value-parity projections such as RNG, actor population, or flag
  subsets; do not approximate parity using topology.
- [x] Expose predicate sampling phase and first-hit boundary unambiguously.
- [x] Provide predicate unit tests against recorded traces and native fixtures.
- [x] Keep goals as optional metadata on segments/runs, never structural parents.

### P0: objective semantics

- [x] Separate feasibility from optimization: satisfy the exact predicate first,
  then minimize simulation ticks or another declared cost.
- [x] Define lexicographic objectives for goal depth, first-hit tick, tape size,
  input complexity, risk, and boundary compatibility.
- [x] Support potential-based shaping from distance, corridor progress, phase, or
  event progress without changing the terminal objective.
- [x] Record every reward component and its source fact so reward bugs are
  inspectable.
- [x] Preserve multiple archive cells for materially different RNG, actors,
  routes, procedures, novelty, or downstream state.

### P0: oracle library

- [x] Reached/avoided stage, room, region, action, animation, flag, actor state,
  or event.
- [x] Collision crossing, OOB, void survival, unexpected load, wrong warp,
  excessive displacement/speed, NaN, and impossible coordinates.
- [x] Actor corruption, slot exhaustion, watched-field corruption, heap failure,
  crash, hang, softlock, and control loss.
- [x] Duplicate item/reward, preserved storage state, cutscene/event queueing,
  sequence break, and save-state anomalies.
- [x] Headful/headless divergence, control/treatment difference, and novel
  semantic event signature.
- [x] Compose cheap per-tick native oracles with expensive Rust-side corpus and
  novelty analysis.

**Acceptance:** representative movement, collision, actor, cutscene, memory,
RNG, transition, crash, and softlock glitches can be classified without visual
judgment, with the exact supporting observations retained.

## 6. Trace v2, corpus, and data quality

### P0: trace v2

- [x] Add an atomic little-endian v2 channel-directory format with explicit
  post-simulation boundaries, input provenance, per-record channel status,
  strict size/offset/canonical validation, and permanent v1 decoding.
- [x] Capture opt-in current/pending stage, exact four-port applied PAD, Link
  motion and raw action/procedure state, event control, both global RNG streams,
  realized camera, and nearest scene exit through the gated observer boundary.
- [x] Make the native writer and Rust decoder reject malformed flags, missing
  requested channels, overlap/gaps, invalid statuses, incompatible RNG, and
  contradictory phase/tape metadata; cover the wire layout with a native test.
- [x] Enforce byte-identical repeated real traces in the intro conformance
  runner; an initial camera-view leak was localized to tick 300 and fixed by
  marking unrealized view state `Unavailable` without copying its payload.
- [x] Replace the fixed 49-field movement assumption with an authenticated,
  extensible observation-view schema and explicit missingness masks while
  retaining permanent movement-v1 compatibility for existing corpora.
- [x] Add per-tick RNG, camera, player procedure internals/action timers,
  contacts, correction vectors, local geometry, goal features, and selected
  dynamic actors.
- [x] Record pre-action observation, exact chosen action/option, duration,
  post-action observation, events, reward components, predicate state, and
  terminal reason with exact phase alignment.
- [x] Support dense state views plus sparse entity/event side tables to avoid
  repeating unchanged world data.
- [x] Add bounded ring buffers and trigger-based retention so a crash, novel
  contact, flag change, or predicate hit can preserve dense evidence from the
  preceding and following ticks without paying full diagnostic trace cost for
  every candidate.
- [x] Preserve raw high-value observations and stable world references so later
  featurizers can be rerun offline.
- [x] Bound record size, actor slots, event counts, and output; report every
  truncation explicitly.

### P0: transition and episode identity

- [x] Give every episode a scenario, parent boundary, absolute tape, run build,
  query view, action schema, objective, learner/proposer, seed, and worker ID.
- [x] Store candidate lineage and intervention location without treating frames
  from one episode as independent provenance.
- [x] Separate successful, failed, crashed, timed-out, desynced, unsupported,
  and truncated runs.
- [x] Deduplicate exact episodes while retaining independent repetition evidence.
- [x] Content-address large traces, static inventories, screenshots, models, and
  crash artifacts.

### P0: counterfactual collection

- [x] At incumbent and archive routes, choose decision boundaries and evaluate
  systematic alternate actions/options, including failures.
- [x] Track coverage by stage/room, spatial cell, player procedure, option,
  parameter bin, duration, goal phase, and outcome.
- [x] Balance collection across underrepresented supported actions instead of
  repeatedly perturbing only successful headings.
- [x] Let ensemble disagreement request bounded probes, but record that it is a
  heuristic rather than calibrated uncertainty.
- [x] Retain random/Latin-hypercube probes for blind spots and audit for policy
  collapse.
- [x] Make the evaluator budget and attribution per proposer visible.

### P0: train/validation/test discipline

- [x] Split by whole episode, scenario, parent boundary fingerprint, and route
  family; never randomly split correlated frames.
- [x] Keep a frozen withheld benchmark suite that model selection cannot farm.
- [x] Report unique episodes, effective decision count, action support, state
  coverage, missingness, class imbalance, and boundary diversity—not only rows.
- [x] Detect train/evaluation leakage through duplicated tapes, prefixes,
  checkpoints, screenshots, or continuation ancestry.
- [x] Provide success/failure sibling trace diffing by phase, event, actor,
  contact, flag, RNG draw, allocation, and objective component to expose facts
  a model or predicate is currently missing.
- [x] Version normalization statistics and compute them from training data only.
- [x] Store dataset manifests and exact model-training configuration in Git or
  content-addressed immutable artifacts.

### P1: corpus operations

- [x] Query, compare, merge, shard, compact, re-feature, validate, quarantine,
  and garbage-collect corpora by schema and identity.
- [x] Delta-debug a successful tape while retaining its oracle and boundary
  class.
- [x] Prune unreachable generated artifacts and orphaned thumbnails safely;
  provide dry-run and recoverable trash for user-owned route data.
- [x] Export analysis-friendly Arrow/Parquet or equivalent outside the hot path
  without making it replay authority.

**Acceptance:** a training manifest can be reproduced from immutable episodes,
reports actual action/state coverage, contains both success and counterfactual
failure evidence, and cannot leak one route's frames across dataset splits.

## 7. Deterministic and sample-efficient search baselines

Serious RL must beat credible specialists, not a deliberately weak random
mutator.

### P0: exact and structured methods

- [x] Exhaustive menu-pulse timing search, coordinate descent, chunk deletion,
  delta debugging, and tape truncation.
- [x] Deterministic roll-spacing, heading, magnitude, waypoint, spline, button
  timing, and option-duration optimizers.
- [x] Beam search and branch-and-bound over discrete options with exact simulator
  rollouts.
- [x] Cross-entropy method and CMA-ES for low-dimensional continuous option
  parameters.
- [x] Bayesian optimization for very expensive, smooth-enough bounded tactics.
- [x] Novelty search and quality-diversity/MAP-Elites across route, behavior,
  RNG, actor, contact, and boundary descriptors.
- [ ] MCTS over checkpointed option boundaries where restore is validated.

### P0: fair proposer tournament

- [x] Give each proposer a declared candidate-tick or episode budget.
- [x] Deduplicate proposals before spending simulator time.
- [x] Attribute every improvement, miss, crash, and duplicate to its proposer.
- [x] Compare wall time, simulator ticks, episodes, predicate-hit rate, frame
  wins, boundary diversity, and cold-replay pass rate.
- [x] Retain incumbent mutation and blind exploration budgets even when a learned
  proposer appears strong.
- [x] Do not allow any proposer to bypass the same native predicate and replay
  gates.

**Acceptance:** every learned-method claim includes equal-budget comparisons to
exact deletion/golf, structured tactic optimization, archive exploration, and a
simple random baseline.

## 8. Serious RL and planning program

### P0: keep the low-data baseline honest

- [x] Retain deterministic tree FQI and document its supported observation/action
  schemas, uncertainty limitations, and episode bootstrap behavior.
- [x] Add nearest-neighbor/local return and tabular discretization baselines for
  small objective-specific state spaces.
- [x] Add n-step and option-duration targets with tests for terminal and truncated
  episodes.
- [x] Calibrate predictions against held-out simulator returns and proposal win
  rate rather than training Bellman loss alone.

### P1: offline conservative value learning

- [x] Implement a small discrete Double-Q learner with target networks and
  deterministic seeded training.
- [x] Add discrete Conservative Q-Learning to penalize unsupported actions.
- [x] Add Implicit Q-Learning plus advantage-weighted behavior cloning as a
  dataset-constrained alternative.
- [x] Add bootstrapped/twin/ensemble critics and episode-level resampling.
- [x] Add prioritized replay with bounded importance correction and diagnostics.
- [x] Implement isolated, equal-update-budget evaluators for dueling heads,
  n-step returns, distributional values, and noisy exploration.
- [ ] Evaluate those four components one at a time on a readiness-qualified,
  content-disjoint corpus before adopting a Rainbow configuration.
- [x] Mask structurally unavailable actions for efficiency while retaining an
  explicit exploration path for nominally invalid inputs that may cause bugs.

### P1: action hierarchy and goal conditioning

- [x] Train option-level values before per-frame neural control; keep raw actions
  for last-mile frame golf.
- [x] Factor tactic, heading, magnitude, duration, target, and button overlay so
  sparse combinations can share statistical strength.
- [x] Add goal-conditioned value/policy inputs using a compiled objective vector
  rather than a model hard-coded to one segment.
- [x] Evaluate hindsight relabeling only for predicates whose semantics make the
  relabeled transition valid.
- [ ] Support a high-level option policy with a deterministic low-level tactic
  executor and realized-tape proof.

### P1: active online collection

- [ ] Alternate conservative exploitation, ensemble-disagreement probes,
  structured counterfactuals, archive novelty, and blind coverage.
- [ ] Cap update-to-data ratio and detect critic divergence or value explosion.
- [ ] Use independent evaluation workers/corpora so online training cannot turn
  proof repetitions into training samples before evaluation completes.
- [ ] Resume training with immutable dataset generations and exact model lineage.
- [ ] Stop or fall back when supported-action or state coverage is inadequate.

### P1: model representation

- [ ] Begin with normalized fixed features, missingness masks, categorical
  embeddings, objective vector, nearest-K semantic actor slots, and local
  geometry probes.
- [ ] Test short history stacking and recurrent critics when state remains
  partially observable after Trace v2.
- [ ] Add DeepSets/attention over variable actor sets only when fixed slots fail
  and the corpus is large enough.
- [ ] Evaluate graph encoders for actor relationships and local collision graphs
  only against simpler representations under equal sample budgets.
- [ ] Keep static map geometry in a spatial service or local encoder; never feed
  the complete raw mesh to an MLP every tick.

### P1: model ownership and deployment

- [ ] Keep worker orchestration, corpora, scheduling, and promotion in Rust.
- [ ] Permit Python/PyTorch as an offline trainer between generations; it is not
  a per-frame or process-orchestration dependency.
- [ ] Export inference artifacts through ONNX or another frozen, versioned format
  for Rust/C++ batch inference.
- [ ] Compare inference in Rust versus native worker only after measuring IPC and
  batching costs; do not put a network in the game tick by default.
- [ ] Hash feature schema, action schema, objective, normalization, code/data
  build, corpus manifest, seed, optimizer, and model bytes.
- [ ] Add deterministic CPU inference tests and tolerance-declared accelerator
  tests.

### P2: planning and model-based research

- [ ] Use learned Q values as priors/heuristics for beam search or MCTS rather
  than requiring the policy to own the whole route.
- [ ] Learn short-horizon local dynamics only after measuring prediction error
  on contacts, procedures, RNG-sensitive branches, and actor interaction.
- [ ] Investigate Dyna-style real/model rollout mixtures with strict uncertainty
  cutoffs.
- [ ] Consider latent visual/world models only for observations unavailable from
  memory or for console-transfer fidelity; memory-backed state should remain the
  sample-efficient default.
- [ ] Research multi-task and transfer learning across compatible maps, tactics,
  and goal families without merging incompatible fidelity or action schemas.

### RL readiness gates

- [ ] At least 500 diverse episodes and 50,000 option decisions for the selected
  objective before treating a neural comparison as meaningful.
- [ ] Broad action/option support in each relevant player procedure and spatial
  phase, with unsupported regions reported explicitly.
- [ ] Held-out episodes and boundary families, stable repeated cold replay, and a
  stronger result than tree FQI plus structured specialists under equal budget.
- [ ] Value calibration and OOD-action diagnostics good enough that proposals do
  not spend most of the simulator budget on unsupported fantasies.

The numeric gates are initial engineering heuristics and should be revised from
measured learning curves. They are deliberately based on diverse episodes and
decisions, not raw consecutive frame count.

**Acceptance:** CQL/IQL/Double-Q and the existing FQI compete through one
proposer interface; their models and datasets are reproducible; and promoted
improvements are independently proved absolute tapes.

## 9. Novelty and autonomous glitch discovery

- [ ] Define semantic novelty descriptors over procedures, events, contacts,
  transitions, actor relationships, flags, position/velocity extrema, and
  boundary fingerprints.
- [ ] Detect first-seen state transitions and rare state combinations, not just
  spatial distance from an incumbent.
- [ ] Maintain separate archives by scenario/fidelity and allow several useful
  outcomes per behavior cell.
- [ ] Add curiosity/novelty rewards only as proposal signals; retain the raw
  semantic reason a run was considered novel.
- [ ] Cluster similar crashes, hangs, OOB routes, corruptions, and event sequences
  to avoid rediscovering the same symptom endlessly.
- [ ] Minimize novel artifacts while preserving the novelty predicate and replay
  boundary.
- [ ] Automatically replay promising headless discoveries headfully, attach a
  terminal thumbnail/video when useful, and request human classification.
- [ ] Feed human labels back as corpus metadata, never as a silent rewrite of
  prior objective definitions.
- [ ] Support campaigns that ask open questions such as “produce an unseen
  procedure/contact pair” or “cross collision without a transition.”

**Acceptance:** a discovery run produces a ranked, deduplicated set of exact
artifacts with machine-readable novelty reasons, rather than a directory of
uninspectable random tapes.

## 10. Experimental causal interventions

- [ ] Implement canonical `DUSKINTR` plus a readable bounded DSL, separately
  from controller tapes.
- [ ] Require compile-time feature, runtime opt-in, fidelity flag, exact phase,
  preconditions, and an always-on before/write/after audit.
- [ ] Begin with typed position, velocity, facing, bounded curve, target/intent,
  health, timer, flag, and spawn/despawn experiments only where semantics are
  understood.
- [ ] Reuse exact stable selectors; reject target loss and overlapping writes.
- [ ] Run identical no-intervention controls and retain both artifacts.
- [ ] Add parameter search/minimization for intervention timing and magnitude.
- [ ] Mark results as existence/mechanism evidence until normal input reproduces
  the setup.
- [ ] Keep arbitrary address writes in a separately named unsafe lab build, if
  they are ever added at all.

**Acceptance:** an enemy-push/fence experiment can establish a causal collision
possibility with complete provenance, while a normal build cannot execute or
mislabel the intervention.

## 11. Skybook replication and capability discovery

The current `..\skybook` checkout contains roughly 483 posts, including
about 452 categorized glitch pages. Its tags already span movement, collision,
cutscenes, OOB, warps, memory, storage, softlocks, combat, crashes, actor
corruption, RNG, platforms, and dozens of maps. This is a requirements corpus,
not merely a list of tapes to transcribe.

### P0: import a read-only benchmark manifest

- [ ] Record the Skybook Git revision and parse front matter, title,
  description, category, tags, internal links, platform, map, source links,
  images, and video evidence.
- [ ] Generate a content-addressed manifest without editing or depending on
  Skybook at runtime.
- [ ] Normalize aliases while retaining original names and source paths.
- [ ] Map each page to required scenarios, actions, observations, oracles,
  fidelity, and known/unknown setup steps.
- [ ] Track `untriaged`, `unsupported`, `scenario-ready`, `observable`,
  `reproduced`, `minimized`, and `discovery-withheld` independently.
- [ ] Link benchmark definitions back to the exact Skybook source revision.

### P0: benchmark specification

For every selected glitch, retain:

- [ ] Region/platform/fidelity and game version.
- [ ] Scenario/save/inventory/flag/RNG preconditions.
- [ ] Human-readable setup and known input timing.
- [ ] Relevant actors, placements, geometry, triggers, flags, procedures,
  timers, resources, and memory facts.
- [ ] A semantic success oracle plus failure/softlock/crash classifications.
- [ ] Exact tape/controller/intervention ancestry and minimized proof.
- [ ] Headful evidence where the effect is visual.
- [ ] Open hypotheses and missing toolbox capabilities.

### P0: representative capability ladder

- [ ] Menu/name-entry corruption: Eye Shredder / file-name cursor breakout.
- [ ] Frame-perfect basic movement: roll timing, Epona slide, item/button timing.
- [ ] Collision/OOB: step/ledge/floor clips, displacement clipping, seam clips,
  crawlspaces, doors, ceilings, water, and load-zone boundaries.
- [ ] Actor interaction: enemy pushes, carried pots, boomerang, clawshot,
  mounts, spawned/destroyed actors, target/aggro manipulation.
- [ ] Combat/damage/death: knockback, invulnerability, simultaneous death/load,
  boss state, dropped items, and revival.
- [ ] Cutscene/event/UI storage: dialogue, map/item delay, camera ownership,
  queued events, control lock, and retained state.
- [ ] Transition/warp: wrong warps, double loads, entrance resolution, void/load
  races, save warps, Ooccoo, and room unload behavior.
- [ ] Memory/heap/process: actor corruption, slot exhaustion, allocation failure,
  stale flags, backing-layout effects, crashes, and console reset interactions.
- [ ] RNG-sensitive: enemy behavior, drops, cycles, particles, and encounter
  manipulation.
- [ ] Visual/audio/platform-specific: renderer corruption, camera, missing
  effects, music manager, GC/Wii/HD differences, and native-port limitations.

### P1: mechanism graph

- [ ] Build a many-to-many graph from glitches to mechanics, prerequisites,
  actors, items, maps, flags, actions, oracles, and fidelity capabilities.
- [ ] Use the graph to prioritize toolbox features that unlock the most pages.
- [ ] Reuse shared benchmark modules for common mechanisms rather than coding one
  custom observer per page.
- [ ] Select withheld known glitches for rediscovery campaigns after their
  scenario and oracle exist, but before their exact tape is ingested.
- [ ] Record negative results and fidelity blockers so unsupported pages do not
  consume repeated search campaigns.

### P1: unknown and unseen state

- [ ] Preserve rich raw actor, flag, contact, event, resource, and map references
  around every surprising run so later analysis can ask new questions.
- [ ] Add schema/version negotiation so new typed facts do not invalidate old
  facts or silently reorder neural inputs.
- [ ] Support targeted native adapters and offline derived features without
  redesigning the worker protocol for each glitch.
- [ ] Add differential traces between near-identical success/failure runs to
  identify candidate fields, actors, contacts, or event edges.
- [ ] Provide bounded address/layout diagnostics for reverse engineering while
  keeping portable proof based on semantic facts.

**Acceptance:** the framework can report, for every Skybook page, whether it has
the required fixture, actions, observations, oracle, and fidelity. Adding a new
page usually means composing existing toolbox pieces; genuinely missing facts
become explicit adapter tasks rather than hidden folklore.

## 12. Workbench, inspection, and human workflow

- [ ] Keep one graphical segment hierarchy with ordinary sibling alternatives;
  avoid separate model, sample, milestone, and generated-result trees.
- [ ] Project active and completed search results as ephemeral segment siblings,
  with proposer, score, proof, boundary, and model/corpus identity in details.
- [ ] Promote, rename, delete subtree, keep/delete siblings, and recover trash
  with previews and Git-visible changes.
- [ ] Record a child from an explicitly selected clean process boot, stage boot,
  or exact parent boundary using the fastest validated prefix/checkpoint tier
  and configurable host-only countdown. Display the selected boot origin and
  fixture identity before recording begins.
- [ ] Play from process boot, stage boot, parent, or parent-fast; capture a
  thumbnail automatically only when the selected boundary has no reachable
  image.
- [ ] Add objective editor/query builder with syntax validation, schema
  discovery, trace preview, and proof invalidation explanation.
- [ ] Add run dashboard for worker health, throughput, budget, proposer
  attribution, action/state coverage, archive cells, model training, and errors.
- [ ] Add side-by-side candidate comparison: input diff, option intervals,
  objective progress, events, state divergence, path/contacts, and terminal
  boundary.
- [ ] Allow visual scrub through recorded observations and thumbnails without
  pretending arbitrary tick playback is a validated checkpoint.
- [ ] Make generated data appear/disappear with the underlying build artifacts;
  keep checked-in segments authoritative.
- [ ] Keep settings compact and global where appropriate: playback speed,
  recording speed, visual/headless, audio, countdown, and capture policy.

**Acceptance:** a user can direct an experiment, watch progress, inspect why a
candidate was interesting, replay it, record a continuation, and promote or
discard it without using a separate algorithm-specific workflow.

## 13. Multi-client and distributed execution

- [ ] Treat each client as an isolated worker with its own scenario, inputs,
  state, artifacts, and crash boundary.
- [ ] Add deterministic logical barriers and an explicit cross-client message,
  delay, loss, and delivery schedule.
- [ ] Record sender, receiver, logical tick, ordering, payload digest, and network
  schedule as replay inputs.
- [ ] Compose multi-port and multi-client goals/oracles.
- [ ] Schedule CPU affinity, memory budgets, worker capabilities, NUMA locality,
  and thermal/throughput telemetry based on measurements.
- [ ] Add a single-host coordinator before distributed execution.
- [ ] Add distributed immutable artifact transport, deduplication, leases, and
  failure recovery only after one host saturates usefully.
- [ ] Never let different build/game-data/fidelity workers share a training or
  proof pool without explicit compatibility rules.

**Acceptance:** a coordinated artifact replays the same ordered cross-client
event stream, and increasing worker count changes wall-clock throughput rather
than logical outcome.

## 14. Testing, safety, and operational quality

### P0: native and schema tests

- [ ] Golden C++/Rust codecs for every binary artifact and protocol version.
- [ ] Property/fuzz tests for parsers, bounded allocations, malformed lengths,
  compression bombs, path traversal, and schema mismatch.
- [ ] Unit tests proving observation capture does not mutate state or advance
  RNG.
- [ ] Controller composition, option termination, target loss, action mask,
  and realized-tape round-trip tests.
- [ ] Predicate phase, temporal operator, missingness, and oracle tests.
- [ ] Checkpoint restore and validation-window corruption tests.

### P0: end-to-end conformance

- [ ] Clean-boot repetition suites for menu, movement, transition, actor,
  collision, RNG, crash, and recording/handoff paths.
- [ ] Stage-boot conformance suites spanning representative overworld, dungeon,
  boss, interior, grotto, and cutscene maps, with small goal-specific tapes
  proving fixture readiness, input alignment, and isolated save/loadout state.
- [ ] For representative stage-boot tests, promote the local result into a
  clean-process absolute replay when claiming end-to-end route capability;
  report local fixture validation and full boot proof as distinct evidence.
- [ ] Headful/hidden/headless parity and first-divergence reports.
- [ ] Memory-card/save isolation and persistent-worker leak tests.
- [ ] Worker crash/timeout/restart and partial-pool failure tests.
- [ ] Route graph CRUD, generated-result projection, thumbnail lifecycle, and
  recoverable deletion tests.
- [ ] Search reproducibility and equal-budget proposer attribution tests.

### P1: performance and regression gates

- [ ] Track candidate-ticks/second, reset latency, checkpoint latency, trace
  bytes/tick, query cost, model inference cost, and IPC overhead.
- [ ] Establish representative trace profiles from minimal predicate-only to
  full diagnostic capture.
- [ ] Fail CI on substantial deterministic throughput or artifact-size
  regressions only after stable machine-normalized baselines exist.
- [ ] Stress long tapes, large maps, actor churn, event storms, many workers,
  crashes, and disk exhaustion.

### P1: security and failure containment

- [ ] Loopback-only authenticated workbench sessions and no browser-supplied
  arbitrary paths.
- [ ] Canonicalize every artifact path inside configured roots.
- [ ] Bound native queries, trace output, model inputs, worker memory, process
  count, and wall/tick time.
- [ ] Make intervention capabilities visible in UI, artifacts, logs, and build
  identity.
- [ ] Preserve enough failed-run evidence for diagnosis without accepting
  partial/corrupt proof.

**Acceptance:** ordinary tests can be launched from the existing VS Code test
selector or CLI, failures retain actionable evidence, and no malformed corpus,
query, model, or browser request can create unbounded native work.

## Explicit over-engineering traps

- Do not build an end-to-end pixel DDQN while memory-backed state is available
  and the counterfactual corpus is small.
- Do not send observations or actions through Rust IPC once per frame; upload a
  bounded program/query and return batched artifacts.
- Do not feed the complete map mesh or every live actor directly to a fixed MLP.
  Keep the complete inventory queryable and compile task-local views.
- Do not make checkpoints portable truth. Absolute tapes and clean replay remain
  truth; checkpoints are invalidatable acceleration caches.
- Do not create a general-purpose native scripting or arbitrary-memory query
  engine. Prefer bounded typed adapters and compiled query specifications.
- Do not build another VCS or algorithm-specific result hierarchy. Git owns
  promoted segments; rebuildable content-addressed stores own experiments.
- Do not add distributed orchestration or exotic shared-memory transport until
  warm local workers are measured and saturated.
- Do not let experimental interventions contaminate ordinary playback, model
  evaluation, or promotion evidence.
- Do not claim global TAS optimality. State the exact operator neighborhood,
  scenario, fidelity, observation/action schema, and simulator budget searched.

## 15. Recommended implementation sequence

### Phase A: trustworthy observation substrate

- [ ] Finish run identity, async determinism audit, canonical state hashes, and
  first-divergence reporting.
- [ ] Design the typed fact/query schema and static world-inventory format.
- [x] Implement the first Trace v2 slice for Link, global RNG, camera, action
  timers/animations, exact input, stage/event state, and explicit missingness.
- [ ] Extend the initial Trace v2 background-collision cache with actor/push/
  attack contacts, per-pass correction, local geometry,
  objective state, and bounded selected actors.
- [ ] Add query/trace cost accounting and observational-inertness tests.

### Phase B: reusable action and objective toolbox

- [ ] Implement option schema, option diagnostics, move/align/roll/waypoint/
  spline/actor-seek tactics, and deterministic composition.
- [ ] Extend predicates and oracles over the new facts/events.
- [ ] Compile options to realized tapes and prove them through cold replay.

### Phase C: high-information data collection

- [ ] Land engine-session batch runs and validated prefix/checkpoint reuse.
- [ ] Add systematic counterfactual collection and coverage reports.
- [ ] Split immutable datasets by episode and boundary; establish frozen
  evaluation routes.
- [ ] Run equal-budget FQI, structured, exact, novelty, and random baselines.

### Phase D: conservative neural learning

- [ ] Add small discrete Double-Q, CQL, IQL, ensemble uncertainty, prioritized
  replay, and option-level goal conditioning.
- [ ] Export frozen inference artifacts and use one proposer interface.
- [ ] Compare sample-efficiency curves and actual simulator proposal wins.
- [ ] Introduce recurrent or object-set models only in response to measured
  partial observability or representation failure.

### Phase E: Skybook-scale campaigns

- [ ] Generate the Skybook capability/benchmark manifest and mechanism graph.
- [ ] Build the representative capability ladder before attempting every page.
- [ ] Run withheld rediscovery, minimization, fidelity classification, and
  headful review campaigns.
- [ ] Use recurring missing capabilities to drive new toolbox adapters rather
  than one-off benchmark hacks.

### Phase F: open-ended discovery and scale

- [ ] Add semantic novelty archives, causal interventions, model-assisted
  planning, and human classification.
- [ ] Expand checkpoint tiers, multi-client determinism, and distributed workers
  only after their preceding correctness gates pass.

## Immediate next milestone

The active implementation milestone is **completing Trace v2 plus the query
contract**, not “DDQN exists.” The channel-directory wire contract and initial
const-only Link/RNG/camera/action slice now exist; the remaining vertical slice
is:

1. [x] A versioned objective-specific observation specification.
2. [ ] Extend exact `SCENE_EXIT` actor volumes and the initial cached Link
   background-collision channel with polygon exit metadata, actor contacts,
   per-pass correction, local geometry, and selected actor slots.
3. [x] A static `F_SP103`/Ordon world inventory with stable collision, placement,
   exit, and trigger IDs.
4. [x] Whole-episode trace extraction with explicit missingness and exact action
   phase alignment.
5. [ ] Systematic counterfactual probes around the current `ToOrdonSprings`
   incumbent, including misses.
6. [ ] Episode/boundary dataset splits and an action/option coverage report.
7. [ ] Equal-budget comparison of forest FQI, structured roll/waypoint optimization,
   and a small conservative Double-Q/CQL prototype.

That slice immediately improves inspection, predicates, manual TAS work,
search, and future Skybook reproduction. It also creates the first dataset on
which a neural Q method can be evaluated honestly.
