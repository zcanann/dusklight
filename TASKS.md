# Learning framework work queue

## Purpose

Track only work required to build a generic framework that actually learns from
native experience and does so fast enough to use. This includes learning,
save-state execution, observations and actions, orchestration, introspection,
and the architecture needed to trust and extend them.

Ordon is the first proof, not the product. A route of 123 native ticks or fewer
is evidence of success, never a value to encode. Authored routes, waypoints,
route-shaped rewards, blessed tactics, proxy terminals, favorable-seed mining,
and benchmark-specific exceptions do not count as learning.

## Done means

- Scratch reliably beats frozen-policy and random-valid controls on repeated,
  predeclared, budget-matched held-out Ordon experiments.
- Learning keeps improving after first success and produces a 123-tick-or-faster
  route that cold-replays twice with identical inputs, state identity, terminal
  proof, and tick count.
- A matched standard Ordon comparison finishes within ten minutes on the named
  two-worker Windows host; more workers predictably increase unique useful
  transitions per second.
- The unchanged framework discovers and improves a second native route.

## Current evidence

Canonical comparison V43 (`530e544671681f137f16c90f39bc5010554fb6ef69e765e74842257dd2062588`)
found 1/2 terminals at 407 ticks with learned ranking and 0/2 with both controls.
That is encouraging, not proof. Learned ranking was also the slowest cell: 368.6
seconds, 0.694 useful expansions/second, and 39% two-worker utilization. We need
repeatability and throughput before spending more time on large searches.

## Ordered work

### P0 - Trust the loop and the baseline

- [ ] Prove end to end that new native evidence passes through observation,
  legal actions, exploration, replay, update, and snapshot publication to change
  future choices. Prove frozen and random controls cannot receive that benefit.
- [ ] Run repeated one-worker and two-worker versions of the same predeclared
  workload. Report outcome, wall-time, and unique-useful-throughput variance and
  identify the largest reconciled loss in the learned cell.
- [ ] Make the learning path auditable: split oversized runner/orchestration
  files by responsibility, enforce source-size and dependency boundaries, and
  cover each contract with focused unit, integration, fault, and replay tests.

### P1 - Make useful experience cheap

- [ ] Optimize the largest measured throughput loss with matched before/after
  evidence. Repeat until the standard comparison meets the ten-minute budget
  and two workers outperform one in unique useful transitions per second.
- [ ] Prove save-state capture, restore, reuse, and branching save time. Account
  for restore latency, fallback replay, duplicate descendants, memory, and useful
  work per state; redesign or remove machinery that does not pay for itself.
- [ ] Eliminate invisible waste and operational fragility. Account for every
  duplicate, stale, censored, discarded, retried, or failed sample; bound and
  backpressure owned processes, workers, queues, memory, replay, checkpoints,
  and artifacts; make progress, ETA, cancellation, cleanup, recovery, and resume
  exact.

### P2 - Make the learner solve and optimize sparse-terminal routes

- [ ] Give exploration enough horizon and breadth to discover a human-reachable
  terminal from scratch. Compare by useful native experience, not arbitrary
  decision counts or short trials that cannot reach the goal.
- [ ] Expose route-independent motion history, velocity, orientation, camera,
  analog input, legal prompts, action availability, and transition outcomes.
  Retain primitive inputs while learning, composing, promoting, and retiring
  parameterized multi-frame tactics from evidence rather than authorship.
- [ ] Validate a learner that propagates sparse terminal value, escapes local
  optima, tolerates replay staleness, and continues exploration and updates after
  first success. Optimize terminal success first and authenticated ticks second;
  use ordinary human replay only as optional off-policy experience.
- [ ] Pass the repeated held-out Ordon comparison and twice cold-replay an
  authenticated 123-tick-or-faster learned route.

### P3 - Harden and generalize

- [ ] Make concurrent publication, sampling, updates, and snapshots logically
  deterministic. Fresh and resumed runs must reproduce decisions, identities,
  accounting, and terminal evidence before adding concurrent lanes.
- [ ] Keep hot-path and durable state compact, bounded, binary, versioned,
  checksummed, atomic, and migration-tested. Reject corrupt, partial,
  incompatible, or inconsistent state before it reaches learning.
- [ ] Pass the same scratch discovery, controlled evaluation, optimization, and
  replay gates on a second native route without changing framework contracts.

## Queue rules

- Work in priority order unless a later item directly unblocks the current one.
- The native load-zone predicate is the only success authority.
- Every experiment answers one falsifiable question; every optimization retains
  matched evidence and auditable build/input identity.
- Remove completed items. History belongs in commits and sealed artifacts.
