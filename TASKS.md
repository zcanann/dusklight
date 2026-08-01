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

- Development seeds: `104729`, `155921`. Held-out seeds: `130363`, `181081`.
- The corrected matched development comparison is valid and causally complete
  (`build/benchmarks/ordon-native-matched-dev-seeds104729-155921-d32-v1-comparison-accounting-v2.json`,
  content `048c1024...6278f`). Learned reached the real terminal on 2/2 seeds;
  frozen-policy and random-valid reached 0/2.
- Learned required a 123.36-second median and 170 useful graph expansions to
  first terminal. Total unique useful work was closely matched: learned 254,
  frozen 255, random 256. This proves a development advantage, not held-out
  generalization or route quality.
- The one predeclared held-out comparison also passed
  (`build/benchmarks/ordon-native-matched-heldout-seeds130363-181081-d32-v1-comparison-v4.json`,
  content `df75f271...0b1f1`). Learned reached 2/2 terminals at a 119.24-second
  median and 166 useful expansions to first terminal; both controls reached
  0/2 under closely matched work. Across all four predeclared seeds, learned
  is 4/4 and both controls are 0/4. The learning claim is established at this
  scope, but route quality remains poor: the best development and held-out
  routes were 304 and 286 ticks versus the 125-tick human replay.
- Critical-path and additive occupancy are now reported separately. The
  held-out learned critical path is 331.47 seconds: 105.72 seconds waiting on
  proposal execution, 99.22 seconds still unattributed, 64.59 seconds of
  persistence, 45.93 seconds of learner updates, 9.65 seconds of launch, and
  6.33 seconds of known orchestration. The parts reconcile exactly; the
  unattributed 29.9% is the first P1 instrumentation defect to repair.

## Queue

Complete these gates in order. When a gate fails, repair the first demonstrated
cause instead of starting a larger campaign.

### P1 - Make it fast enough to use

- [ ] Eliminate the 99.22-second held-out learned critical-path attribution gap.
      Separate learner refresh, action-catalog construction, graph scheduling
      and leasing, seed setup/finalization, campaign setup/finalization, and
      final report persistence/shutdown. Require checked reconciliation to the
      actual completion-marker wall; never hide overlap or missing time with
      saturating arithmetic.
- [ ] Establish a reproducible one-worker baseline and bounded scaling curve.
      Track useful transitions/second, terminal samples/time, CPU, memory,
      queues, duplicate work, and idle capacity.
- [ ] Optimize the largest measured bottleneck and retain matched before/after
      evidence. Scale only while useful throughput improves without changing
      learning semantics.

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
