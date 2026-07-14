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
# Build the game without code mods when not running in a VS developer shell
cmake --preset windows-clang-debug -DDUSK_ENABLE_CODE_MODS=OFF
cmake --build --preset windows-clang-debug --target dusklight

# Native, game-data-free tests
cmake --build --preset windows-clang-debug --target dusk_input_tape_test
cmake --build --preset windows-clang-debug --target dusk_input_controller_test
cmake --build --preset windows-clang-debug --target dusk_game_clock_test
cmake --build --preset windows-clang-debug --target dusk_name_entry_observer_test
cmake --build --preset windows-clang-debug --target dusk_name_entry_trace_test
cmake --build --preset windows-clang-debug --target dusk_rng_test

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
- Eye Shredder's original-layout corruption can be modeled safely, but the
  console-only rendering result is not emulated or claimed.
- The Aurora automation change is committed inside the submodule on a local
  `dusklight-automation` branch. A public fork of Aurora must contain that commit
  before the parent Dusklight commits can be cloned elsewhere.
