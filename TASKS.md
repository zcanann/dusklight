# Ordon route optimizer

This is the active task list. It exists to answer one question:

> Can Dusklight improve the current Link-control-to-Ordon-Springs segment,
> faster and more reliably than a human editing the tape frame by frame?

This is not a catalog of eventual glitch-hunting features. Add work only when
the current benchmark demonstrates that it is necessary.

## Non-negotiable rules

- A playback mismatch is a framework defect. Do not search for a tape that
  happens to tolerate the mismatch.
- Score simulation ticks only. Startup, prefix replay, rendering, shaders,
  training, and host I/O never become route time.
- Normal runs may read game state and supply controller input. They may not
  mutate gameplay state.
- Checkpoints are an acceleration mechanism, not proof. A winning result is an
  ordinary absolute input tape replayed from boot.
- Search and learning may rank candidates. Only the terminal predicate and
  cold playback can establish success.
- Keep the hot per-tick loop native. Rust may configure campaigns and retain
  results, but it must not mediate every frame.

## 1. Freeze and measure the benchmark

- [x] Define the exact source boundary at first controllable Link input and the
  exact terminal predicate for entering Ordon Springs.
- [x] Retain the current human tape as the named incumbent.
- [x] Add one repeatable benchmark command that reports useful candidate ticks
  per second, prefix ticks, process launches, simulator idle time, CPU use, and
  bytes/files written.
- [ ] Record the incumbent's position, speed, facing, camera, applied input,
  collision correction, action state, roll state, and predicate progress each
  tick.
- [ ] Report elementary route defects numerically: excess distance, heading
  error, collision loss, corner duration, and roll timing.

The frozen boundary is `tolink_link_control` at fingerprint
`ac7c32788fc3b5c59046386d95b9b5b4`; the terminal authority is
`ordon_spring_load_committed`. The retained human incumbent is
`intro/segments/to_ordon_spring_human150.tape`. The checked route benchmark and
its first measured Windows baseline are documented in
`docs/glitch-hunting/throughput.md`.

**Exit:** we can compare later changes against an unchanged route, objective,
and workload rather than relying on visual impressions.

## 2. Make repeated experiments cheap

- [ ] Keep one game process alive for a batch of attempts. Load the disc,
  immutable resources, and prefix once per worker.
- [ ] Capture one same-process checkpoint at the source boundary after proving
  that the engine is safe to pause there.
- [ ] Restore all state that can affect future simulation, including game
  memory, RNG, clocks, controller state, and relevant host-side engine state.
- [ ] Run A/B/A tests from the checkpoint and require both A executions to
  produce identical per-tick hashes and predicate evidence.
- [ ] Compare checkpointed attempts with fresh boot-and-prefix attempts.
- [ ] On any mismatch, stop and retain the first divergent tick. Do not admit
  samples from that worker afterward.
- [ ] Batch attempts in memory and return compact results. Do not emit routine
  traces, databases, logs, or screenshots per candidate.
- [ ] Measure full checkpoint copies first; optimize restoration only if the
  measured copy cost limits useful throughput.

**Exit:** one worker can run 1,000 short suffix attempts without relaunching or
replaying the prefix, with zero A/B/A or cold-process divergence.

## 3. Put decisions beside the simulation

- [ ] Add a native episode loop at the pre-input tick boundary:
  observe, choose input, advance one tick, score, repeat.
- [ ] Upload a policy or tactic program once per batch. No per-frame IPC,
  filesystem traffic, or Rust callback is allowed in the hot path.
- [ ] Preserve the exact four-port controller state consumed on every tick and
  emit it as an ordinary tape after the attempt.
- [ ] Allow cancellation, timeout, health checks, and clean worker replacement
  between attempts.
- [ ] Keep cold boot playback as a separate, unchanged verifier.

**Exit:** raw tapes and stateful policies use the same native tick boundary and
produce byte-identical realized tapes when they choose the same inputs.

## 4. Build only the movement toolbox this route needs

Observations:

- [ ] Link position, velocity, speed, facing, action/procedure, action phase,
  roll eligibility/recovery, previous input, and collision correction.
- [ ] Camera position and heading, stage-transition state, relevant timers,
  and enough RNG identity to detect a bad restore.
- [ ] Read-only target/load-zone geometry and small local queries such as
  direction, clearance, raycast, and predicted contact.

Actions:

- [ ] Exact raw pad state and precisely timed button edges.
- [ ] World-, player-, and camera-relative heading for a bounded duration.
- [ ] Seek a coordinate or opening, with configurable gain and stop condition.
- [ ] Line, waypoint, and Bezier stick paths compiled to exact per-tick input.
- [ ] A roll action with explicit initiation, heading, recovery, and
  termination rather than an arbitrary held-A window.
- [ ] Deterministic composition of movement, camera, and button actions, with
  explicit ownership when layers overlap.

Do not encode the incumbent's coordinates, headings, corner ticks, or roll
spacing into these primitives.

**Exit:** the existing route can be expressed through generic actions, while
raw pad input remains available when an abstraction is wrong.

## 5. Search from causal comparisons

- [ ] From an identical checkpointed state, evaluate neighboring headings,
  button edges, action durations, roll timings, and tactic parameters.
- [ ] Retain sibling identity so the optimizer can distinguish action effect
  from a different starting state or RNG lineage.
- [ ] Use terminal success and first-hit tick as authority. Keep any shaped
  progress score separate and inspectable.
- [ ] Establish equal-candidate-tick baselines for simple input deletion,
  local frame search, structured tactic search, and continuous parameter
  optimization.
- [ ] Add a replay buffer and value learner only after causal samples exist.
  The first learned method should be the smallest one that can beat those
  baselines; its value is measured by native candidate ordering, not loss.
- [ ] Allow short checkpoint-backed lookahead so a useful early change is not
  rejected merely because the old continuation needs repair.
- [ ] Keep distinct successful terminal states when they may have different
  downstream value; fastest is not automatically best outside this benchmark.
- [ ] Keep simulators busy while proposal generation or training proceeds.

**Exit:** under the same useful-simulation budget, the best automated lane
consistently orders and finds improvements better than local tape mutation.

## 6. Promote one real win

- [ ] Convert the best candidate to an absolute boot-to-terminal tape.
- [ ] Remove redundant inputs only when exact replay preserves the terminal
  predicate, first-hit tick, and terminal fingerprint.
- [ ] Audit adjacent input edges, headings, durations, corner timing, and roll
  timing so an obvious local improvement is not left untested.
- [ ] Replay from cold boot five times with identical per-tick hashes and
  terminal evidence.
- [ ] Publish the incumbent and winner ticks, useful simulator budget,
  throughput, checkpoint cost, methods compared, and where the gain came from.

**Exit:** a cold-replayable tape reaches Ordon Springs multiple ticks earlier
than the retained human incumbent, and the improvement is attributable to
tested movement decisions rather than nondeterminism.

## Execution order

1. [ ] Freeze and measure the benchmark.
2. [ ] Implement persistent attempts and the source checkpoint.
3. [ ] Prove checkpoint determinism against cold execution.
4. [ ] Move observation and control into the native tick loop.
5. [ ] Implement the minimal movement toolbox.
6. [ ] Run causal local and structured baselines.
7. [ ] Add learning only where the baselines leave a measured gap.
8. [ ] Promote and cold-prove the first multi-tick win.
