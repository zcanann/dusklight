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

- [x] Capture the source state once inside a persistent game process.
- [x] Restore all state that affects the next tick: emulated memory, native
  mutable state, clocks/RNG, VI, controller history, and tape position.
- [x] Prove A/B/A suffix identity with per-tick hashes.
- [x] Prove checkpointed A matches a fresh boot-and-prefix A.
- [x] On mismatch, stop at and report the first divergent component and tick.
- [x] Run 1,000 short attempts in one process without replaying the prefix or
  writing an artifact per attempt.
- [x] Measure copy/restore cost before considering incremental snapshots.

**Gate:** zero divergence across the 1,000-attempt test and the cold-run
comparison.

- Gate evidence: `build/checkpoint-probe-macos-v38/result.json` completed 1,000
  trusted restores with no divergence. The 294,694,644-byte image took
  1,787,506 microseconds to capture; median trusted restore was 4,856
  microseconds (the two fully validated restores are retained in the same
  series).
- Independent cold processes `v39` and `v40` each passed exact raw A/B/A for a
  20-tick suffix and produced the same process-independent replay digest
  `e5a24f7534eb42c1097cf6f4c73ed016`. Their raw checkpoint sequence digests
  differ, as expected for process-local native addresses.

## 2. Run candidate batches at the input boundary

- [x] Add an in-process loop at the point immediately before pad input is
  consumed: restore, apply candidate, advance, evaluate, repeat.
- [x] Batch candidate definitions and compact results in memory; no per-frame
  IPC or filesystem traffic.
- [x] Record the exact consumed pad state so any successful attempt can be
  exported as an ordinary tape.
- [x] Expose only observations needed for this segment: Link position,
  velocity, facing/action/roll state, camera heading, collision correction,
  transition state, previous input, and restore identity.
- [x] Begin with exact pad edits plus timed heading and button/roll edges. Add
  a higher-level action only when a measured search needs it.

**Gate:** a batch candidate and an equivalent raw tape produce identical pad
states, hashes, predicate evidence, and exported tape.

- Gate evidence: `build/suffix-batch-equivalence-v2/result.json` ran an exact
  candidate and raw-tape passthrough from the same captured source. Both hit on
  tick 125 with identical consumed pads, state-sequence digest
  `6f9a1be5cfab33da52428b1b6ec172e5`, and predicate evidence; the exported
  winner is byte-identical to the incumbent segment tape.
- The first 107-candidate search batch completed without IPC or per-frame
  filesystem writes. Every compact terminal result included a finite collision
  correction vector, and median restore cost was 4,922 microseconds.
- Native suffix-batch schema v2 pins the expected source boundary. The fresh
  process in `build/search/source-boundary-v2-valid/result.json` observed the
  expected `ac7c32788fc3b5c59046386d95b9b5b4` fingerprint before checkpoint
  capture and completed its candidate. The paired
  `build/search/source-boundary-v2-rejected/result.json` supplied a different
  fingerprint and failed before checkpoint capture or candidate execution,
  while reporting both expected and actual values.

## 3. Beat manual editing — blocked

- [x] Under equal simulated-tick budgets, compare:
  - deletion and earliest-valid button-edge search;
  - local stick-heading, duration, corner, and roll-timing mutation;
  - one structured or learned candidate ranker if the collected samples can
    support it.
- [x] Judge methods by valid route improvements found, not model labels or
  training loss.
- [x] Use terminal success and first-hit tick as authority. Keep progress
  shaping diagnostic and separate.
- [x] Allow bounded suffix repair when an earlier improvement invalidates the
  old continuation.
- [ ] Preserve materially different successful end states instead of assuming
  the locally fastest one has the best continuation. **Blocked:** no candidate
  reached the terminal predicate within 125 executed suffix ticks, so there is
  no successful end state to preserve.

**Gate:** automation repeatedly finds a valid improvement that local manual
tape editing misses under the same simulated-tick budget.

**Blocked evidence (2026-07-20):** 73 retained passed native batches evaluated
18,867 candidates (2,358,375 simulated suffix ticks) with zero terminal
successes. The initial equal-budget comparison gave deletion, earliest button
edges, heading, corner, timing, and structured ranking 107 candidates / 13,375
simulated ticks each. Later exhaustive and repair batches covered all unique
single-frame deletions, fixed-length delete-and-hold duration shifts, exact
20-tick roll cadence neighbors, local and broad heading windows, corner/contact
windows, symmetric and
asymmetric lane shifts, magnitude/clipping, measured pair/triple combinations,
and 1,000 independent stick-component neighbors. Progress shaping reduced the
signed exit-edge miss from about 11.8 to 2.7813 world units, but shaping never
overrode native success and no candidate reached `ordon_spring_load_committed`
early. Evidence is retained under `build/search/ordon-q124-*`; aggregate counts
come from their passed `result.json` files. Continuing now requires a new,
measured non-local proposal family rather than another obvious local neighbor.
The 2.7813-unit near miss is also checked in as the explicitly unproved
`to_ordon_spring_near_miss_2p781` workbench sibling for visual review; that
review artifact has no promotion authority and does not satisfy gate 4.

A bounded non-local follow-up derived native world-coordinate waypoint policies
from the successful 125-tick incumbent trace while retaining its seven roll
edges. Six cold 125-tick controllers spanning waypoint interval, phase,
lookahead, and terminal extension all reproduced the source fingerprint and
missed the terminal predicate. Their best signed exit-edge distance was
66.6412, far worse than the retained exact-input near miss at 2.7813, so the
family was rejected without adding machinery. Evidence is under
`build/search/ordon-waypoint-*-e500/`.

## 4. Promote the result — blocked by gate 3

- [ ] Export the winner as an absolute boot-to-Ordon-Springs tape.
- [ ] Exhaust its obvious neighboring input timings and headings.
- [ ] Replay it from cold boot five times with identical hashes and predicate
  evidence.
- [ ] Record incumbent/winner ticks, candidate-tick budget, throughput, restore
  cost, methods compared, and the input decisions responsible for the gain.

There is no winner to export or cold-prove. Promotion remains blocked until a
candidate reaches the native terminal predicate in at most 125 executed suffix
ticks (first-hit tick 124 or earlier).

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
