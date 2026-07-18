# Implementation status

This page describes code that exists in the fork today. The architecture and
roadmap documents describe the larger target.

## Working foundations

- `orig/` is ignored as local extracted-game data.
- Aurora exposes an exclusive automation pad source. Owned ports replace
  keyboard, physical, touch, and ordinary virtual input for that tick.
- DUSKTAPE v3.2 has matching C++ and Rust codecs. It zstd-compresses the canonical
  52-byte controller frame stream, stores four exact ports without serializing
  native `PADStatus` memory, authenticates process/stage boot origins and an optional
  canonical scenario descriptor, and still decodes legacy v1/v2 tapes.
- C++ playback performs no per-tick allocation. Recording reserves a declared
  capacity and stops deterministically if it is exhausted.
- Dusklight can load a tape with `--input-tape`, choose release/hold/loop end
  behavior, and exit after the last executed tape tick.
- `huntctl controller compile` turns a bounded timeline DSL into canonical
  `DUSKCTRL` bytes. The native evaluator supports cubic stick curves,
  world/player/camera coordinate seeking, planes, resolved path points and
  inferred openings, turn/brake/neutral/alignment/heading/distance controls,
  exact actor seeking, independently composed main/sub-stick correction,
  post-composition safety clamps, and button overlays without
  allocation in the per-tick path.
- A controller can run alone or continue after a tape. Tape frames have exact
  priority, the controller begins on the following tick, and a headful run
  releases exclusive PAD ownership after completion.
- `--realized-input-tape` records the raw pre-clamp controller output with its
  prefix as an absolute `DUSKTAPE`, preserving the tape as replay authority.
- Proof-anchored roll golf verifies a successful option execution against its
  complete tape, then emits exact one-axis neighbors for heading, magnitude,
  duration, phase, button timing, and cancellation timing.
- Proof-anchored path golf does the same for waypoint, rail, spline, and Bézier
  point coordinates plus duration, sampling phase, and cancellation timing;
  every neighbor carries its exact static tape realization.
- Bounded discrete beam search expands typed options and scores every prefix
  through repeated native rollouts. It deduplicates before launch and applies
  branch-and-bound only to already terminal prefixes whose suffixes are
  provably dominated.
- Seeded bounded CEM and full-covariance CMA-ES optimize declared typed
  move/roll/path axes. Rounded duplicates are removed before launch and every
  update is driven solely by repeated native rollout rank; generation state
  and exact champion artifacts remain auditable.
- `dusklight-option-diagnostic/v1` joins authenticated option boundaries and
  end reason to per-tick target error, guidance-mask decisions, raw output,
  clamp disposition, exact game-consumed input, contacts, target projection,
  and goal progress. Authenticated diagnostic bundles live beside tapes as
  `<artifact>.options.json`; route-workbench graph v8 renders their intervals,
  stick/camera curves, targets, contacts, and progress, including normalized
  target/contact markers over terminal gameplay thumbnails.
- Reactive observation is one-way: immutable player/camera/actor snapshots
  enter the pure evaluator. No gameplay state is written; inactive playback
  does not capture observations or claim the PAD.
- Exact process-ID and placed-actor selectors terminate with typed
  `TargetLost` before emitting the missing-target tick. Incomplete bounded actor
  snapshots never assert false loss, and exact selectors never fall back to a
  nearest actor.
- Authored milestone language 1.2 preserves 1.0/1.1 decode compatibility while
  adding exact placed-actor facts, finite player-distance geometry, and bounded
  event/temporary/dungeon/current-room switch queries. Native evaluation uses
  one immutable phase snapshot; duplicate or truncated actor identity and
  unavailable flag scope evaluate false instead of guessing.
- Milestone language 1.3 adds canonical inclusive ranges, immutable player AABB
  and normalized signed-plane relations, and two-to-sixteen-step ordered
  sequences with mandatory logical-tick windows. `within 1` expresses an exact
  next-tick transition; timeout and overlapping-start behavior are deterministic.
- Milestone language 1.4 adds named, definition-authenticated exact value
  projections over either RNG streams, canonical stage/room actor populations,
  or indexed flags. Native result schema v5 captures inspectable values and a
  canonical value fingerprint at the first-hit observation; Rust comparison
  returns equal, different, or incomparable without consulting route topology.
- Authored predicate regression coverage includes both Rust-produced/native-
  decoded DMSP fixtures and offline evaluation over a normally decoded
  `DUSKTRCE` fixture. Missing trace channels remain unavailable rather than
  acquiring default values.
- Search population/result schemas v3 record canonical compiled input
  complexity and the exact terminal-predicate verdict. Every leaderboard uses
  the declared order: feasibility, progress depth, first-hit tick, tape size,
  input complexity, measured risk, then authenticated boundary compatibility.
  Unknown risk is not zero and route topology cannot stand in for compatibility.
  Missing/inconsistent verdicts and repeated-run projection disagreement are
  rejected before optimization; legacy schemas remain readable.
- Fitted-Q proposal shaping uses authenticated feature-schema-bound distance,
  corridor, phase, and event potentials. It applies
  `gamma^duration * Phi(next) - Phi(source)`, zeroes terminal next potential,
  cannot affect predicate feasibility or leaderboard ordering, and requires a
  versioned sidecar containing every component and its source/next facts.
- Behavior archive v3 is a bounded MAP-Elites table over named RNG and actor
  projections, portable contact trajectories, terminal boundary identity,
  coarse route, procedure sequence, and complete downstream state. Trace-backed
  cells also bind run-deduplicated event, semantic state-transition, actor-
  relationship, flag, and quantized position/velocity-extrema axes. The
  versioned raw semantic descriptor remains inspectable and excludes native
  session process IDs. One native quality elite occupies each cell;
  farthest-first selection preserves novel cells while the normal evaluator
  remains promotion authority.
- Semantic novelty catalog v1 detects exact first-seen state transitions and
  low-support aligned state/event/contact/actor/flag combinations independently
  of route distance. Support is counted once per episode, and assessments retain
  the raw facts and prior episode counts used by the decision.
- Discovery archive v1 partitions cells by exact scenario and headless/headful
  fidelity identities. Each semantic cell retains several distinct useful
  outcome classes, with bounded deterministic same-class replacement instead of
  a single winner silently erasing crashes, OOB routes, or other outcomes.
- Semantic novelty proposal signals v1 are capped, componentized, and retain
  their raw novelty assessment. They may order proposals but explicitly have no
  native-score, proof, or promotion authority.
- Symptom cluster index v1 groups repeated crash, hang, OOB, corruption, and
  event-sequence discoveries by stable semantic context. Volatile addresses and
  process IDs are excluded; counts grow while retained examples stay bounded.
- Novelty minimization v1 freezes raw first-seen/rare facts and an exact replay
  boundary before bounded tape deletion. No reduction replaces the artifact
  unless replay preserves both contracts, and every attempt remains auditable.
- Semantic oracle schema v1 classifies reached/avoided stage, room, region,
  procedure/mode, animation, indexed flag, placed-actor state, and event
  targets. Avoidance requires complete known coverage; truncated or unavailable
  channels produce an inspectable indeterminate result rather than a guess.
- The same oracle evaluator detects plane crossing without declared contact,
  OOB, bounded void survival, unexpected loads, wrong warps, excessive
  displacement/speed, non-finite state, and impossible coordinates from exact
  adjacent records or bounded logical-tick windows.
- Versioned run-outcome evidence closes the trace failure gap for actor/field
  corruption, actor-slot exhaustion, heap failure, crash, wall-time hang,
  semantic-progress softlock, and bounded control loss. Per-domain monitoring
  declarations are mandatory before absence can prove an avoided result.
- The same bounded sidecar classifies duplicate item/reward grants, state which
  survives a required storage reset, cutscene/event queues, progression sequence
  breaks, and slot-specific save-state anomalies with exact expected/actual
  source facts.
- Comparison-oracle schema v1 finds the first exact headful/headless or
  control/treatment semantic divergence and detects event signatures absent
  from a SHA-256-identified reference catalog. Equivalence and non-novelty
  require complete event streams; observed differences remain valid on a
  truncated prefix.
- Oracle composition validates bounded native monitor observations, converts
  them to timing-independent typed semantic hashes with separate tick/tape
  provenance, includes terminal outcomes, and binds the resulting comparison
  evidence to a canonical event-catalog identity. Corpus joins and novelty
  analysis remain offline Rust work rather than per-tick game-loop work.
- Transition evidence schema v1 binds each compact learner record to its source
  corpus, trace, and tape. It retains exact pre/post simulation boundaries,
  complete four-port input or typed option execution, duration, event and goal
  progress, reward-component provenance, and a typed terminal reason. Manual
  extraction and anchored episode farming emit the sidecar automatically;
  frame shifts, duplicate observations, detached identities, and unexplained
  terminal transitions are rejected. Dense learner views remain in the compact
  corpus while exact event and selected-actor snapshots are interned into
  sparse side tables with pre/post indices, so unchanged world facts are not
  repeated per transition.
- DUSKTRCE v5 adds opt-in trigger retention without changing dense recording.
  A pre-trigger circular buffer and bounded post-trigger window retain exact
  samples around identity-aware new contacts, semantic flag changes, predicate
  hits, or an explicit controlled-crash trigger. The header authenticates the
  configured and observed trigger masks, pre/post sizes, trigger count, total
  observed ticks, and capacity exhaustion; Rust permanently decodes v1-v5.
- Learner corpora remain disposable authenticated views over immutable raw
  trace/tape sources. Evidence binds both source digests, while Trace channels
  retain exact RNG words, procedures/timers, camera, collision code/geometry,
  exit destinations, process IDs, and placed-actor identities. Re-running
  `learn extract-trace` can therefore apply a later featurizer without trying
  to invert normalized dense features.
- Trace output is bounded to 131,072 fixed-stride records in both native and
  Rust code; oversized dense requests fail before the game loop and recommend
  trigger retention, while hostile files are rejected before record-vector
  allocation. Actor payloads retain at most 16 stable-ID entries with exact
  observed count and a truncation bit, scene-exit count saturation is flagged,
  each channel has an explicit availability/truncation status, retention
  declares omitted ticks, and output exhaustion is carried in the file header.
- Episode manifests bind scenario/fixture boot, parent boundary, absolute tape,
  executable, query/action schemas, objective, producer, seed, worker, lineage,
  structured intervention, outcome, trace, corpus, and transition evidence.
  Evaluation ledgers group identical inputs, count exact duplicate episodes,
  and retain independently hashed attempt proofs as repetition evidence.
- A bounded SHA-256 content store covers gameplay traces, static world
  inventories/indices, screenshots, serialized fitted-Q models, and crash
  diagnostics. Human-friendly paths remain, but reports carry verified blob
  references and identical bytes occupy one immutable path.
- Anchored counterfactual collection samples incumbent and behavior-archive
  decision boundaries with separately budgeted systematic, fitted-Q,
  disagreement, deterministic-random, and Latin-hypercube proposers. Coverage
  and collapse audits retain action/state/outcome support, and disagreement is
  explicitly labeled as heuristic uncertainty.
- Dataset manifests build leakage-safe whole-episode train/validation/test and
  frozen-withheld splits by unioning scenario, boundary, route, tape/prefix,
  checkpoint, screenshot, and ancestry relationships. Reports include support,
  coverage, missingness, imbalance, boundary diversity, and training-only
  versioned normalization statistics; fitted models bind the dataset and
  ordered corpus identities.
- Typed sibling trace diffing finds the first phase, event, actor, contact,
  flag, RNG-draw, selected-process-population, and objective/reward difference,
  while explicitly reporting unavailable heap-allocation fidelity.
- Schema-aware corpus commands query, compare, deduplicate/merge, compact,
  shard, re-feature from immutable evidence, validate, and quarantine batches.
  Content GC verifies identities, previews by default, reports missing roots,
  and moves unreachable artifacts to explicit recoverable trash rather than
  deleting them.
- Route-store object collection and Route Workbench thumbnail pruning are now
  explicit preview/apply operations. Both move unreachable generated data into
  transaction-scoped recoverable trash, and ordinary graph browsing no longer
  deletes cache entries.
- Offline Apache Arrow IPC export provides typed analysis columns plus input
  and output digests. Its file metadata and sidecar explicitly declare that it
  is not replay authority; authenticated transition corpora remain canonical.
- `--actor-catalog PATH` emits a bounded read-only JSON snapshot at the
  automation endpoint before headful handoff. `DUSKCTRL` 1.3 can seek the
  nearest actor of a type, an exact session process ID, or a placed actor by
  stage, type, home room, and map-authored set ID; exact selectors never fall
  back to nearest. Version 1.2 carries resolved path/opening identity and
  position; version 1.3 adds bounded motion-control primitives and version 1.4
  adds camera layers and safety clamps while retaining strict 1.0-1.3 decode
  compatibility.
- `--unpaced` produces one logical 30 Hz tick per outer loop and removes VSync,
  interpolation timing, and the frame limiter.
- Fixed-step modes also replace `OSGetTime`/`OSGetTick` with an atomic logical
  clock and advance it exactly once per completed simulation tick. Rational
  stepping retains fractional phase without drift; the initial tick defaults
  to zero and can be set with `--deterministic-time-start`.
- The two process-global `c_math` random streams expose versioned snapshots,
  transactional restore, and draw counters. Capturing or restoring them does
  not allocate and preserves the original Wichmann-Hill output sequence.
- `--headless` forces Aurora's null renderer, keeps the game/GX traversal, and
  hides its required SDL window. It requires an explicit DVD path and
  never falls back into the prelaunch UI.
- The native executable exposes a persistent bootstrap worker with versioned
  build/capability discovery, ping, structured errors, and clean shutdown.
- Rust `huntctl` supervises the native worker and strictly verifies protocol,
  request IDs, build identity, and capabilities.
- `huntctl` can keep an N-worker pool alive, enforce identical-build or explicit
  mixed-build policy, schedule parallel coarse health jobs, retain healthy
  workers after partial startup failures, and shut every accepted worker down.
- `huntctl tape compile` compiles the concise `.tas` state DSL into canonical
  compressed tape bytes; `huntctl tape inspect` decodes tapes. Markers live in
  a sidecar so replay bytes stay exact. Legacy JSON input remains readable but
  is not used by checked tapes.
- `huntctl corpus` stores immutable tapes and run metadata under real SHA-256
  names, uses atomic replacement for manifests, and verifies blobs against
  corruption or tampering.
- `huntctl learn` stores authenticated binary transition batches, runs a native
  deterministic tree-based fitted-Q learner, and extracts explicitly
  non-authoritative per-frame movement transitions from absolute tape/trace
  pairs with correct post-tick alignment. Each extraction also emits a
  digest-bound transition-evidence sidecar with exact phase and provenance.
- Typed roll plans compile direction, an exact B frame, bounded recovery, and
  absolute modulo spacing phase into raw frames. Pre-input typed cancellation
  truncates the realized range exactly, and capture authenticates it through
  the common option-execution/tape contract. Search roll macros use this same
  planner.
- Typed game-tactic plans cover jump attacks, attacks/combos, shield/target,
  interaction, explicit-slot item use and named items, transform, crawl/climb/
  swim movement, and Epona movement/spurs. Static search macros compile them to
  exact raw tapes; adaptive cancellation uses authenticated execution capture.
- A versioned reusable tactic-test catalog labels exact `PROC_*` and mode-flag
  contexts, covers all 15 tactic families across human, wolf, horse, crawl,
  climb, and swim modes, and checks hand-authored PAD samples without confusing
  static recipe conformance with in-game acceptance proof.
- Exact static motion paths cover waypoint holds, linear rails, Catmull–Rom
  splines, and cubic Béziers with rational sample phase, integer interpolation,
  defined rounding, exact duration, search macros, and authenticated capture.
- Name-entry instrumentation exposes stable logical/visual cursor snapshots,
  original-layout offsets, and a bounded event ring. Its off-by-default Eye
  Shredder shadow profile models original big-endian neighboring writes without
  native out-of-bounds access.
- `--name-entry-trace` writes the final snapshot, modeled bytes, counters,
  dropped-event count, and ordered events as versioned JSON. The explicit
  `--cursor-breakout-shadow` switch requires that trace output.

## Useful commands

```console
# Read-only observation build without code mods
cmake --preset windows-clang-debug -DDUSK_ENABLE_CODE_MODS=OFF \
  -DDUSK_ENABLE_AUTOMATION_OBSERVERS=ON \
  -DDUSK_ENABLE_AUTOMATION_FIDELITY_MODELS=OFF
cmake --build --preset windows-clang-debug --target dusklight

# Static fork/native observation-boundary guardrail
python tests/automation_boundary_test.py

# Native, game-data-free tests
cmake --build --preset windows-clang-debug --target dusk_input_tape_test
cmake --build --preset windows-clang-debug --target dusk_input_controller_test
cmake --build --preset windows-clang-debug --target dusk_game_clock_test
cmake --build --preset windows-clang-debug --target dusk_name_entry_observer_test
cmake --build --preset windows-clang-debug --target dusk_name_entry_trace_test
cmake --build --preset windows-clang-debug --target dusk_rng_test
cmake --build --preset windows-clang-debug --target dusk_gameplay_trace_test

# Rust tests and lint
cargo test --manifest-path tools/huntctl/Cargo.toml
cargo clippy --manifest-path tools/huntctl/Cargo.toml --all-targets -- -D warnings

# Compile and inspect a reactive timeline
cargo run --manifest-path tools/huntctl/Cargo.toml -- controller compile \
  tests/fixtures/automation/intro_seek_forward.duskctl build/intro-seek-forward.dctl
cargo run --manifest-path tools/huntctl/Cargo.toml -- controller inspect \
  build/intro-seek-forward.dctl

# Build and inspect the boot authoring smoke tape
cargo run --manifest-path tools/huntctl/Cargo.toml -- tape compile \
  tests/fixtures/automation/boot_start_smoke.tas build/boot-start-smoke.tape
cargo run --manifest-path tools/huntctl/Cargo.toml -- tape inspect \
  build/boot-start-smoke.tape

# Initialize, ingest into, and verify a content-addressed corpus
cargo run --manifest-path tools/huntctl/Cargo.toml -- corpus init build/corpus
cargo run --manifest-path tools/huntctl/Cargo.toml -- corpus ingest \
  build/corpus build/boot-start-smoke.tape
cargo run --manifest-path tools/huntctl/Cargo.toml -- corpus verify build/corpus

# Probe the real persistent native worker
cargo run --manifest-path tools/huntctl/Cargo.toml -- hello \
  --worker build/windows-clang-debug/dusklight.exe \
  --worker-arg --automation-worker

# Exercise a two-process persistent pool
cargo run --manifest-path tools/huntctl/Cargo.toml -- pool health \
  --worker build/windows-clang-debug/dusklight.exe \
  --worker-arg --automation-worker --workers 2 --checks 6

# Play a tape visibly or through the null renderer
dusklight --dvd game.iso --input-tape run.tape --exit-after-tape
dusklight --headless --dvd game.iso --input-tape run.tape --exit-after-tape
dusklight --headless --dvd game.iso --input-tape prefix.tape \
  --input-controller route.dctl --exit-after-controller \
  --realized-input-tape build/route-realized.tape
dusklight --headless --dvd game.iso --input-tape eye-shredder.tape \
  --exit-after-tape --deterministic-time-start 0 --cursor-breakout-shadow \
  --name-entry-trace eye-shredder.trace.json
```

## Validated limits

- The Windows Clang debug game target builds and links.
- Native tape, reactive-controller, fixed-step, name-entry observer/trace, and
  exact RNG sequence and round-trip tests pass.
- Trace v2 writes atomic channel-directory artifacts with explicit status and
  post-simulation boundaries. Its native golden-layout test, strict v1-v5 Rust
  decoder, and offline phase/provenance guards pass.
- Per-tick trace coverage includes both global RNG streams, realized camera,
  Link procedure internals/timers/animations, contacts, correction vectors, and
  collision-backed local geometry. Goal channel 11 records same-boundary
  predicate progress after evaluation. Opt-in actor channel 12 retains 16
  non-player actors by lowest session ID with exact observed count and explicit
  truncation; native and Rust decoders enforce its 656-byte layout and canonical
  unused slots.
- Aurora's deterministic time tests pass for exact/rational stepping,
  concurrent reads, reset phase, overflow, and unchanged realtime defaults.
- Rust formatting, all tests, and warning-clean Clippy pass.
- Corpus tests cover deduplication, immutable manifests, and tamper detection.
- Rust-to-native persistent hello/ping/shutdown works against the real
  executable.
- Two real native control workers complete round-robin parallel health jobs and
  shut down cleanly under the Rust pool.
- Null-backend initialization and deterministic failure for an invalid explicit
  DVD were exercised.

A real `GZ2E01` run validated tape-to-controller handoff at route-control frame
439. The 45-tick controller continued in `F_SP103`, produced a 485-frame
realized tape, and a fresh absolute-tape replay reached identical final map,
position, velocity, and applied input telemetry. Broader headful/headless parity
and throughput remain unmeasured.

The real `intro-first-exit` absolute tape also produced three byte-identical
925-record Trace v2 artifacts across independent cold headless runs (Link
control 439, collision-surface proof 826, trigger 827, `F_SP104` load 858).
Channel 10 proves the transition through cached KCL prism 2217, PLC attribute
19, exit 1, and the exact room-1 SCLS destination; its raw code and geometry
indices match the independently parsed content-addressed room archive. The
first conformance attempt
exposed varying uninitialized camera-view bytes at tick 300; the observer now
marks unrealized camera state `Unavailable`, and the runner fails if complete
trace SHA-256 values ever disagree again.

The offline bridge now has an authenticated `movement-state/v2` observation
artifact for the F_SP103-to-F_SP104 objective. It binds exact Trace channel
formats and status policies to 98 stable typed fields spanning stage/target
state, Link motion and action, applied PAD, event availability, scene-exit
geometry, cached ground collision/exit, correction, RNG, and camera state.
Semantic absence is mask-distinct from zero; unavailable or truncated required
facts fail extraction. `huntctl observe` emits/inspects the canonical spec, and
v2 extraction writes a matching spec sidecar while preserving movement-v1
compatibility.

The offline world reader now builds canonical
`dusklight-world-inventory/v1` artifacts directly from immutable
RARC/DZS/DZR/KCL/PLC content. The checked F_SP103 fixture inventories 1,442
placements, 48 player spawns, 44 SCLS exits, 10,794 collision prisms, and 40
inferred collision-to-SCLS load triggers. Its artifact SHA-256 is
`370675af90d40e5b6d8e17b8dce3ad48873bec74c7f7c05bb69b50de95201e7f`.
Four degenerate retail prisms remain addressable with explicit reconstruction
failures. The content-golden test independently proves that room-1 prism 2217,
PLC attribute 19 and exit 1 join room-1 SCLS record 1 and `F_SP104` room 1.
This path is offline and read-only; no native or gameplay file participates.

The inventory now feeds a canonical per-room median-AABB BVH and bounded
coordinate inspector. The checked F_SP103 spatial artifact contains 10,790
reconstructed triangles and four explicit degeneracy exclusions; its SHA-256
is `2ad975eee45193b4325bb420a7ba5a78d533bed80cbcfeace29dcc5418e73834`.
`huntctl world query point|aabb|ray` requires an authored room coordinate scope,
applies exact trigger/destination filters before ranking, returns stable source
facts, and reports truncation plus traversal cost. Real goldens prove the live
transition coordinate and a filtered ray resolve prism 2217 / `F_SP104`, while
an unfiltered point deliberately resolves nearer prism 2187. The service is
offline Rust only and introduces no native or gameplay changes.

## Known gaps

- `OSAlarm` dispatch, the separate SDK `__OSGetSystemTime`, and non-SDK host
  clocks are not driven by logical time yet. Busy waits on `OSGetTime` require
  the simulation driver to keep advancing ticks.
- Loading, audio, movie, and other asynchronous threads have not been made
  deterministic.
- Actor-local `cM_rnd_c`, JMath/Z2 static RNGs, and manager-owned RNG instances
  are not part of the global RNG snapshot yet.
- The persistent worker does not yet own an engine session or accept run
  batches; current tape execution uses process CLI options.
- Headless still creates an invisible SDL window because Aurora's null renderer
  requires a presentable surface to drain the existing GX traversal.
- There are no canonical state hashes or reset/checkpoint acceleration yet.
- Actor observation is currently bounded to 256 snapshots, with deterministic
  retention and stable-ID tie-breaking. Controller programs have no internal
  state transitions yet, and actors are selected by numeric process name.
- Actor catalogs embed build identity but not a game-data/scenario digest yet;
  placed selectors must only be reused with independently validated game data.
- Eye Shredder's original-layout corruption can be modeled safely, but the
  console-only rendering result is not emulated or claimed.
- The Aurora automation change is committed inside the submodule on a local
  `dusklight-automation` branch. A public fork of Aurora must contain that commit
  before the parent Dusklight commits can be cloned elsewhere.
