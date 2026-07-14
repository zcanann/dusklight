# Implementation roadmap

Each milestone ends with a runnable acceptance test. Later search work should
not paper over missing determinism in earlier layers.

## 0. Fork baseline and capability inventory

- Keep fork-specific design and compatibility code isolated and documented.
- Emit build identity, Aurora identity, target, flags, game-data digest, and
  fidelity profile at startup.
- Add a machine-readable capability report.
- Inventory `TARGET_PC`, `AVOID_UB`, timing, input, and render conditionals that
  affect known Skybook glitches.

Acceptance: a run artifact can prove exactly which executable and fidelity
profile produced it.

## 1. Deterministic boot tape

- Add a callable single-tick driver around the main loop.
- Add exclusive per-port input selection.
- Define the canonical tape schema, recorder, player, and simple inspector.
- Add semantic UI observations and bounded `wait-for` controller operations.
- Implement the Eye Shredder stages through cursor breakout analysis.

Acceptance: the Eye Shredder boot artifact replays 100 times with identical
events and hashes and can be viewed headful.

## 2. Logical time and replay parity

- Replace game-visible wall time with a deterministic logical clock in worker
  modes.
- Expose RNG state/call counters and deterministic initialization.
- Audit asynchronous loading, audio, movie, and job completion ordering.
- Build canonical state hashes and first-divergence diagnostics.

Acceptance: realtime and unpaced execution of the same tape agree at every
declared checkpoint.

## 3. True headless worker

- Allow the null/render-sink backend to continue running the game without a
  window.
- Remove retrace and device pacing while preserving logical ticks.
- Preserve draw traversal until parity tests prove individual work removable.
- Add terminal conditions, watchdog heartbeat, and structured crash output.

Acceptance: headless and headful runs agree on hashes/events, and headless shows
a measured candidate-tick throughput improvement.

## 4. Rust control plane

- Create `huntctl` and a shared versioned protocol crate/schema.
- Launch and supervise persistent workers.
- Batch candidates, collect artifacts, restart crashes, and support CPU
  affinity.
- Add replay promotion and a corpus CLI: list, inspect, replay, minimize, and
  compare.

Acceptance: one command runs a corpus across N workers and promotes a selected
result into a headful client without changing its tape.

## 5. Scenario fixtures and reset acceleration

- Formalize save/stage/room/entrance fixtures and semantic ready conditions.
- Cover the map catalog with smoke scenarios.
- Add deterministic prefix replay and measure reset cost.
- Prototype explicit game-state checkpoints only for validated subsystems.

Acceptance: any supported map fixture loads to the same initial hash, and every
checkpoint passes its validation replay.

## 6. Gameplay observation and oracle library

- Stabilize actor identities and expose player, actor, collision, transition,
  animation, and UI observations.
- Add sparse events and common spatial, state, crash, hang, and corruption
  oracles.
- Convert representative Skybook glitches into benchmarks: an exact analog
  technique, a frame-perfect movement technique, collision/OOB, RNG-sensitive
  behavior, and a stage-transition exploit.

Acceptance: each benchmark has a minimized tape and machine-evaluated semantic
oracle, not only a visual assertion.

## 7. Search primitives

- Add tape mutation, timing-window search, delta debugging, and corpus
  minimization.
- Add parameterized stick arcs/splines, roll-spacing programs, and
  observation-feedback controllers.
- Treat the first bounded reactive-controller implementation as a delivered
  substrate, then add typed observations and explicit state transitions only
  when a benchmark requires them. Keep all observation access read-only and
  promote discoveries through realized absolute tapes.
- Add novelty signatures over events and quantized semantic state.
- Retain structured enumeration and exact reduction as promotion tools. Use the
  native fitted-Q transition corpus and a return-and-explore archive to propose
  candidates only after trace-v2 state and episode-level validation exist.

Acceptance: the system rediscovers at least one withheld known glitch from a
coarse scenario/program and produces a smaller deterministic artifact.

## 8. Multi-client and advanced acceleration

- Add deterministic logical barriers and replayable network schedules.
- Explore safe process snapshots/forkserver techniques per platform.
- Add distributed corpus coordination only after single-host workers saturate
  available cores.

Acceptance: a coordinated multi-client artifact replays with the same ordered
cross-client event stream, and acceleration never changes its logical result.
