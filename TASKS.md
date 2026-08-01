# Learning framework roadmap

## Mission

Build a generic framework that learns useful behavior from restorable native
state, observations, legal actions, transition outcomes, optional human
experience, and an authoritative terminal predicate. It must learn from scratch,
improve with experience, and run fast enough for useful iteration.

Ordon is the first proving problem, not the product. A route of 123 native ticks
or fewer is evidence that the framework works, not a target to encode into it.
Authored routes, waypoints, shaped route rewards, blessed tactics, proxy
terminals, favorable-seed mining, and benchmark-specific exceptions do not
count as learning.

## Definition of done

- Scratch learning reliably beats frozen-policy and random-valid controls on
  predeclared held-out Ordon seeds under matched budgets.
- Learning continues after discovery and produces a route of 123 ticks or fewer
  that cold-replays twice with identical inputs, state identities, terminal
  proof, and tick count.
- A standard experiment has an interactive turnaround and scales predictably
  within explicit CPU, memory, worker, and storage limits.
- The same framework discovers and improves a second native route without
  learner or contract changes.

## Current reality

- The latest deterministic held-out run found 1/2 terminals with a 251-tick best
  learned route; frozen and random controls found 0/2. That is weak evidence of
  learning, not a robust result.
- An earlier stronger result was invalidated by wall-clock-dependent cross-lane
  publication. Live replay remains single-lane until ordering is deterministic.
- The learned cell took about 375 seconds. Several phases are expensive, but
  current reports cannot yet identify the real useful-throughput bottleneck.

## Ordered work

### P0 — Trust the experiment

- [ ] Emit one self-validating canonical artifact with inputs/build identity,
  terminal proof, success and route-quality curves, useful and duplicate work,
  CPU/memory/queue/worker utilization, failures, and exclusive phase timing.
  Every total must reconcile and missing evidence must fail validation.
- [ ] Prove the complete learning path—observation, legal actions, exploration,
  transition, replay, update, policy, checkpoint—can change future choices from
  new evidence. Prove frozen and random controls cannot receive that benefit.
- [ ] Run repeatable one-worker and bounded multi-worker baselines on the same
  predeclared workload, report variance, and name the largest measured loss of
  unique useful transitions per second.

### P1 — Raise useful throughput

- [ ] Optimize the largest measured loss and retain matched before/after
  evidence. Repeat until framework overhead no longer dominates and added
  workers increase useful—not merely attempted—transitions per second.
- [ ] Prove save-state restore and branching pay for themselves. Measure restore
  and branch latency, state reuse, unique descendants, duplicate suppression,
  and useful work per restored state; redesign or remove machinery that loses.
- [ ] Bound and backpressure workers, native processes, queues, memory,
  checkpoints, replay, and artifacts. Make cancellation, timeout, failure,
  cleanup, and resume exact; manage only directly owned processes.

### P2 — Make the learner solve and optimize

- [ ] Give exploration enough temporal reach and branching breadth to complete
  sparse-terminal attempts, with explicit comparable budgets rather than short
  horizons that cannot reach the goal.
- [ ] Expose route-independent native facts and choices: motion history,
  velocity, camera, analog input, legal prompts, action availability, and
  transition outcomes. Let learning determine predictive value; do not encode
  “straight”, “wall”, “roll”, coordinates, or the human path as route rewards.
- [ ] Learn reusable parameterized multi-frame actions while retaining primitive
  inputs. Discover, compare, compose, promote, and retire tactics from evidence;
  do not require authored movement or camera sequences.
- [ ] Continue exploration and updates after first success, optimizing terminal
  success first and native tick count second. Report quality versus useful
  experience and wall time, not only the best sample.
- [ ] Evaluate human replay as optional off-policy experience under matched
  budgets. Scratch must remain viable and learning must be able to surpass it.
- [ ] Demonstrate repeatable held-out Ordon success over both controls, then
  produce and twice cold-replay an authenticated route of 123 ticks or fewer.

### P3 — Harden and generalize

- [ ] Define deterministic logical ordering for concurrent publication,
  sampling, updates, and snapshots. Require identical fresh/resumed decisions,
  identities, accounting, and terminal evidence before enabling multiple lanes.
- [ ] Separate execution, restoration, observation/action contracts, learning,
  search, persistence, orchestration, and reporting into focused testable
  modules. Split oversized files and enforce dependency and source-size gates.
- [ ] Keep durable state bounded, binary, versioned, checksummed, atomic, and
  migration-tested. Reject corrupt, partial, incompatible, or inconsistent data
  before it affects learning; make every campaign auditable from artifacts alone.
- [ ] Repeat scratch discovery, controlled evaluation, optimization, and replay
  on a second route without changing learner or framework contracts.

## Rules of work

- The native terminal predicate is the only success authority.
- Predeclare seeds and budgets. Do not mine seeds, rerun unchanged failures, or
  count near misses and proxy thresholds as wins.
- Each experiment answers one question; each optimization needs matched evidence;
  each milestone needs a falsifiable test or sealed artifact.
- Remove completed tasks. Keep history and detailed evidence in versioned reports,
  not in this queue.
