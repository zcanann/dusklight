# Ordon optimizer proof

The active objective is one concrete result:

> Produce a deterministic, cold-boot-replayable tape that reaches Ordon
> Springs faster than the current 125-tick segment, using fewer human hours
> than frame-by-frame TAS editing.

This is an execution queue for that proof. It is not the product roadmap.

## Invariants

- Identical tape plus identical initial state must produce identical per-tick
  state. Any mismatch is a framework bug.
- Route score is simulated ticks to `ordon_spring_load_committed`. Shader,
  rendering, host I/O, prefix replay, and training costs affect throughput but
  never the route score.
- Ordinary playback supplies controller input and may inspect state. It does
  not patch gameplay state.
- A checkpoint may accelerate experiments. It cannot validate a result; only
  repeatable cold-boot playback can.
- Per-tick control stays in the native process. External tooling may configure
  a batch and collect its result, but does not participate frame by frame.
- New machinery must earn its place by improving this benchmark.

## Baseline — complete

- [x] Freeze the first-Link-control source state.
- [x] Define `ordon_spring_load_committed` as the terminal predicate.
- [x] Retain the human reference and 125-tick incumbent.
- [x] Measure cold-run cost and route defects.

- Source fingerprint: `ac7c32788fc3b5c59046386d95b9b5b4`
- Human reference: `intro/segments/to_ordon_spring_human150.tape`
- Measurements: `docs/glitch-hunting/throughput.md`

## 1. Make suffix experiments trustworthy and cheap

- [ ] Capture the source state once inside a persistent game process.
- [ ] Restore all state that affects the next tick: emulated memory, native
  mutable state, clocks/RNG, VI, controller history, and tape position.
- [ ] Prove A/B/A suffix identity with per-tick hashes.
- [ ] Prove checkpointed A matches a fresh boot-and-prefix A.
- [ ] On mismatch, stop at and report the first divergent component and tick.
- [ ] Run 1,000 short attempts in one process without replaying the prefix or
  writing an artifact per attempt.
- [ ] Measure copy/restore cost before considering incremental snapshots.

**Gate:** zero divergence across the 1,000-attempt test and the cold-run
comparison.

## 2. Run candidate batches at the input boundary

- [ ] Add an in-process loop at the point immediately before pad input is
  consumed: restore, apply candidate, advance, evaluate, repeat.
- [ ] Batch candidate definitions and compact results in memory; no per-frame
  IPC or filesystem traffic.
- [ ] Record the exact consumed pad state so any successful attempt can be
  exported as an ordinary tape.
- [ ] Expose only observations needed for this segment: Link position,
  velocity, facing/action/roll state, camera heading, collision correction,
  transition state, previous input, and restore identity.
- [ ] Begin with exact pad edits plus timed heading and button/roll edges. Add
  a higher-level action only when a measured search needs it.

**Gate:** a batch candidate and an equivalent raw tape produce identical pad
states, hashes, predicate evidence, and exported tape.

## 3. Beat manual editing

- [ ] Under equal simulated-tick budgets, compare:
  - deletion and earliest-valid button-edge search;
  - local stick-heading, duration, corner, and roll-timing mutation;
  - one structured or learned candidate ranker if the collected samples can
    support it.
- [ ] Judge methods by valid route improvements found, not model labels or
  training loss.
- [ ] Use terminal success and first-hit tick as authority. Keep progress
  shaping diagnostic and separate.
- [ ] Allow bounded suffix repair when an earlier improvement invalidates the
  old continuation.
- [ ] Preserve materially different successful end states instead of assuming
  the locally fastest one has the best continuation.

**Gate:** automation repeatedly finds a valid improvement that local manual
tape editing misses under the same simulated-tick budget.

## 4. Promote the result

- [ ] Export the winner as an absolute boot-to-Ordon-Springs tape.
- [ ] Exhaust its obvious neighboring input timings and headings.
- [ ] Replay it from cold boot five times with identical hashes and predicate
  evidence.
- [ ] Record incumbent/winner ticks, candidate-tick budget, throughput, restore
  cost, methods compared, and the input decisions responsible for the gain.

**Done:** a repeatable cold-boot tape beats 125 ticks, and the win is caused by
its inputs rather than nondeterminism.

## Not part of this proof

- Replicating the Skybook catalog. Individual glitches may become later,
  explicitly selected benchmarks; they are not a blind checklist.
- A general visualization, observation, or world-inspection workbench.
- Pre-harvesting every actor, object, polygon, or metadata field.
- A mandatory DQN/DDQN/Q-learning stack before simpler measured baselines.
- Distributed execution, elaborate snapshotting, or new UI absent a measured
  bottleneck in the active proof.

After this proof, choose the next benchmark and add only the capability it
demonstrably requires.
