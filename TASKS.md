# Learning framework

## Objective

Build a general system that learns useful behavior from restorable native
states, observations, legal actions, transition outcomes, and authoritative
terminal predicates. It must learn fast enough to iterate on and remain
auditable, deterministic, resumable, and safe to parallelize.

For Ordon, success means discovering the real load zone from scratch, beating
matched frozen-policy and random-valid controls, reaching it within a useful
wall-clock budget, and then improving past the 125-tick human replay to 123
ticks or fewer. The final proof is repeating this on another native route
without route-specific learner changes.

Human input may be optional experience. Authored routes, waypoints,
route-shaped rewards, blessed tactic sequences, proxy terminals, and seed
mining are not learning. TAS graph recording/playback and route-planner UX are
separate product work.

## Current state

- Development seeds: `104729`, `155921`.
- Untouched held-out seeds: `130363`, `181081`.
- The latest matched development run reached the real terminal on both learned
  seeds and neither control arm.
- That result is not accepted yet: its learned report counted 500 shared-graph
  expansions from only 256 completed proposals, so comparison validation
  correctly rejected it.

## Queue

Complete these gates in order. When a gate fails, repair the first demonstrated
cause instead of starting a larger campaign.

### P0 - Prove that learning is real

- [ ] Fix campaign-wide useful-expansion accounting. Shared work counts once,
      independent work remains additive, throughput uses the same authority,
      and impossible totals remain rejected. Add multi-seed regressions.
- [ ] Produce a valid matched development comparison with identical seeds,
      action surfaces, horizons, workers, and resource budgets. Reuse evidence
      only when projection does not alter what happened.
- [ ] Audit the causal chain from restored state through observation, legal
      action, exploration, transition, replay, update, policy deployment, and
      terminal evidence. Every decision and discarded sample must be
      attributable; uncalibrated predictions must have no policy authority.
- [ ] Require learned to beat both controls in development terminal rate and
      sample/time-to-terminal, then repeat the advantage once on the sealed
      held-out seeds. Do not tune against, replace, or mine held-out seeds.

### P1 - Make it fast enough to use

- [ ] Attribute campaign time and utilization to native simulation, save-state
      operations, transport, scheduling/idle time, learning, and persistence;
      reconcile the parts with total time and useful transitions.
- [ ] Establish a reproducible one-worker baseline and bounded scaling curve.
      Track useful transitions/second, terminal samples/time, CPU, memory,
      queues, duplicate work, and idle capacity.
- [ ] Optimize the largest measured bottleneck and retain matched before/after
      evidence. Scale only while useful throughput improves without changing
      learning semantics.
- [ ] Reach scratch discovery in a median of five minutes or less, with no
      retained development or held-out seed above fifteen minutes under a
      documented resource budget.

### P2 - Harden the machinery

- [ ] Separate native execution, restoration, observation/actions, exploration,
      replay/learning, graph search, persistence, orchestration, and reporting
      behind testable boundaries. Split mixed or oversized modules and enforce
      a source-size regression gate.
- [ ] Give campaigns exact ownership of their child processes, limits,
      cancellation, artifacts, and cleanup. Test interruption, timeout, child
      failure, and resume; never discover or kill unrelated processes.
- [ ] Make uninterrupted, replayed, and resumed runs deterministic for the same
      initial state, seed, configuration, and checkpoint, including decisions,
      identities, accounting, and terminal evidence.
- [ ] Keep durable machine state bounded, versioned, binary, checksummed, and
      migration-tested. Reject corrupt, incompatible, partial, or inconsistent
      artifacts before they affect learning.
- [ ] Make every campaign explainable from artifacts alone: inputs, identities,
      legal actions, exploration, samples, updates, snapshots, worker lifecycle,
      timing, failures, and comparison must reconcile from a clean checkout.

### P3 - Prove optimization, not just discovery

- [ ] Continue learning after the first terminal while preserving exploration
      and promoting behavior only from evidence.
- [ ] Expose general movement state, recent trajectory, action legality/prompts,
      analog direction, camera control, roll, and other native actions without
      assigning route-specific rewards to them.
- [ ] Let learned parameterized action compositions compete with primitives and
      be retired when unhelpful; do not encode blessed tactic sequences.
- [ ] Produce an authenticated route of 123 native ticks or fewer and cold-replay
      it twice with identical controller bytes, state identities, terminal
      evidence, and tick count.

### P4 - Prove generality

- [ ] Compare scratch and human-experience-assisted learning under matched
      budgets. Human experience may improve sample efficiency but must be
      optional and must not cap performance.
- [ ] Repeat scratch discovery and post-terminal optimization on a second native
      route without changing learner logic or the observation, action,
      objective, persistence, or orchestration contracts.

## Rules

- The real terminal predicate is authoritative; intermediate signals are
  learner inputs and diagnostics, not hand-authored claims of progress.
- Every experiment answers one named question with the smallest matched run
  capable of answering it. Near misses and arbitrary tick counts are not wins.
- Do not rerun unchanged failures. Correctness and auditability precede scale;
  measured throughput precedes long mining.
- Remove completed work from this file. Commit and push natural milestones and
  do not leave a long-lived dirty workspace.
