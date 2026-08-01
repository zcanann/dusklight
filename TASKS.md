# Learning framework work queue

## Goal

Build a generic system that learns useful behavior from restorable native state,
observations, legal actions, transition outcomes, and authoritative terminal
predicates. It must learn from scratch, improve with experience, and run fast
enough that meaningful experiments take minutes rather than a day.

The Ordon route is the first proving problem, not the product. The learner must:

- discover the real load zone without an authored route;
- outperform frozen-policy and random-valid controls on held-out seeds;
- continue improving after discovery and produce a cold-replayable route of 123
  native ticks or fewer, beating the 125-tick human replay; and
- repeat scratch discovery and optimization on a second native route without
  route-specific learner changes.

Human recordings may be optional experience. They may improve sample efficiency
but must not be required or cap the learned policy. Authored waypoints, shaped
route rewards, blessed tactic sequences, proxy terminals, and favorable-seed
mining do not count as learning. TAS recording/playback and route-planner UX are
separate work.

## Where we are

- Scratch learning has beaten both controls on four predeclared Ordon seeds.
- Route quality is still poor: the best learned route is 286 ticks versus the
  125-tick human replay.
- The deterministic V40/V5 held-out comparison
  (`build/benchmarks/ordon-native-matched-heldout-seeds130363-181081-d32-v40-deterministic-v1-comparison-v5.json`,
  content `30d6cd06...9e4`) is exact and causally complete. Learned reached 1/2
  terminals with a 251-tick best route; both controls reached 0/2. The earlier
  2/2 result depended on cross-lane replay publication order, so new live-replay
  plans are restricted to one lane until deterministic concurrent ordering is
  implemented.
- All three V5 critical paths have zero unattributed time and at most 0.0151%
  timing-boundary residue. The learned cell took 375.337 seconds. Its largest
  top-level phase was orchestration at 118.055 seconds; within that,
  graph scheduling and leasing dominated at 93.889 seconds. Native tactic
  execution was 111.572 seconds, model updates 79.841 seconds, and persistence
  56.429 seconds.

## Next work

Work in this order. Each experiment must answer one question with the smallest
matched run that can answer it. When a gate fails, fix the demonstrated cause
before increasing campaign size.

### 1. Establish a trustworthy baseline

- [ ] Publish one canonical report containing success rate, time and useful
  transitions to first terminal, best route ticks over time, unique useful
  transitions/second, duplicate work, CPU, memory, queueing, and every exclusive
  critical-path phase.
- [ ] Establish reproducible one-worker and bounded multi-worker baselines. Name
  the largest throughput loss and distinguish native execution, restoration,
  learning, search, persistence, scheduling, contention, and idle capacity.

### 2. Make learning fast enough to iterate

- [ ] Optimize the largest measured bottleneck, then repeat the same baseline
  and retain matched before/after evidence. Do not trade away learning semantics,
  determinism, or auditability for headline throughput.
- [ ] Repeat measurement and optimization until added workers increase unique
  useful transitions/second, avoidable framework overhead no longer dominates,
  and a standard matched experiment has a practical interactive turnaround.
- [ ] Make checkpoint restore and branching cheap enough to explore broadly.
  Measure restore latency, branch latency, state reuse, duplicate suppression,
  and useful work per restored state rather than assuming save states help.

### 3. Make the learner improve, not merely stumble into a terminal

- [ ] Preserve exploration and learning after the first terminal. Report success
  probability and best authenticated route length as functions of both useful
  experience and wall time.
- [ ] Audit the full learning loop: observation -> legal actions -> exploration
  -> transition -> replay -> update -> policy. Prove that new evidence can change
  action values and future choices, and that frozen-policy and random-valid
  controls cannot receive those benefits.
- [ ] Surface general, route-independent information needed to learn movement:
  recent trajectory and velocity, action legality/prompts, analog direction,
  camera control, roll, and other native actions. These are observations and
  choices, not hand-authored rewards.
- [ ] Let the learner discover, compare, compose, parameterize, promote, and
  retire multi-frame actions from evidence. Fixed primitives must remain
  available; no human-chosen tactic may be required for success.
- [ ] Evaluate optional human experience under the same budget. It may seed
  useful representations or replay, but scratch learning must remain viable and
  the learner must be able to surpass the demonstration.
- [ ] Produce a route of 123 native ticks or fewer and cold-replay it twice with
  identical controller bytes, state identities, terminal evidence, and tick
  count.

### 4. Keep the framework trustworthy and maintainable

- [ ] Separate native execution, state restoration, observation/action contracts,
  exploration, replay/learning, graph search, persistence, orchestration, and
  reporting behind focused testable interfaces. Split mixed and oversized files
  and enforce a source-size regression gate.
- [ ] Give every campaign exact ownership of its workers, limits, cancellation,
  artifacts, and cleanup. Test interruption, timeout, worker failure, and resume;
  never discover or terminate unrelated processes.
- [ ] Define a logical publication and snapshot-consumption order for concurrent
  learning lanes. Prove identical decisions and learner identities across fresh
  reruns before allowing live replay with more than one lane per generation.
- [ ] Make fresh, replayed, and resumed runs deterministic for the same state,
  seed, configuration, and checkpoint, including decisions, identities,
  accounting, and terminal evidence.
- [ ] Keep durable machine state bounded, versioned, binary, checksummed, and
  migration-tested. Reject corrupt, incompatible, partial, or inconsistent
  artifacts before they influence learning.
- [ ] Make a campaign explainable from its artifacts alone: inputs, identities,
  legal actions, exploration, samples, updates, snapshots, worker lifecycle,
  timing, failures, and comparisons must reconcile from a clean checkout.

### 5. Prove generality

- [ ] Repeat scratch discovery, controlled evaluation, and post-terminal
  optimization on a second native route without changing learner logic or the
  observation, action, objective, persistence, or orchestration contracts.

## Operating rules

- The native terminal predicate is the only success authority. Intermediate
  signals may be learner inputs and diagnostics; they are not declared progress.
- Do not rerun unchanged failures or mine seeds. Diagnose, change one relevant
  thing, and compare under the same budget.
- Measure useful learning throughput, not raw attempts, process count, or CPU
  occupancy. Near misses and arbitrary tick thresholds are not wins.
- Correctness and introspection precede scale. Scale follows evidence.
- Remove completed work from this file. Keep durable evidence in reports, not in
  an ever-growing task history.
