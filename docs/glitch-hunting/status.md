# Implementation status

This page describes code that exists in the fork today. The architecture and
roadmap documents describe the larger target.

## Working foundations

- `orig/` is ignored as local extracted-game data.
- Aurora exposes an exclusive automation pad source. Owned ports replace
  keyboard, physical, touch, and ordinary virtual input for that tick.
- DUSKTAPE v2 has matching C++ and Rust codecs. It zstd-compresses the canonical
  52-byte controller frame stream, stores four exact ports without serializing
  native `PADStatus` memory, and still decodes v1.0-v1.2 tapes.
- C++ playback performs no per-tick allocation. Recording reserves a declared
  capacity and stops deterministically if it is exhausted.
- Dusklight can load a tape with `--input-tape`, choose release/hold/loop end
  behavior, and exit after the last executed tape tick.
- `huntctl controller compile` turns a bounded timeline DSL into canonical
  `DUSKCTRL` bytes. The native evaluator supports cubic stick curves,
  coordinate seeking, actor seeking, additive stick correction, and button
  overlays without allocation in the per-tick path.
- A controller can run alone or continue after a tape. Tape frames have exact
  priority, the controller begins on the following tick, and a headful run
  releases exclusive PAD ownership after completion.
- `--realized-input-tape` records the raw pre-clamp controller output with its
  prefix as an absolute `DUSKTAPE`, preserving the tape as replay authority.
- Reactive observation is one-way: immutable player/camera/actor snapshots
  enter the pure evaluator. No gameplay state is written; inactive playback
  does not capture observations or claim the PAD.
- `--actor-catalog PATH` emits a bounded read-only JSON snapshot at the
  automation endpoint before headful handoff. `DUSKCTRL` 1.1 can seek the
  nearest actor of a type, an exact session process ID, or a placed actor by
  stage, type, home room, and map-authored set ID; exact selectors never fall
  back to nearest.
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
  pairs with correct post-tick alignment.
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
  post-simulation boundaries. Its native golden-layout test, strict v1/v2 Rust
  decoder, and offline phase/provenance guards pass.
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
- There are no canonical state hashes, scenario fixtures, reset/checkpoint
  acceleration, or gameplay oracle library yet.
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
