# Learning framework backlog

## Objective

Build a route-independent native learning framework that can discover a terminal,
learn from the experience it collects, and improve routes quickly enough to be
useful.

Ordon is the first acceptance test, not the design target. The learner may consume
native observations, legal actions, transition history, terminal outcomes, and
optional demonstrations. It may not consume authored routes, waypoints,
route-shaped rewards, blessed action sequences, proxy terminals, or
benchmark-specific exceptions.

## Framework gates

Ordon is complete only when all of these are true:

- Repeated, predeclared, budget-matched scratch runs beat frozen-policy and
  random-valid controls on held-out seeds.
- A learned route reaches the native load-zone predicate in 123 ticks or fewer
  and cold-replays twice with identical inputs, state identity, terminal proof,
  and tick count.
- The standard learned/control comparison completes within ten minutes on the
  named two-worker Windows host.
- Two workers produce meaningfully more unique useful transitions per second
  than one worker on the same workload.

## Work queue

Work top to bottom unless a later task is required to unblock the current one.
Remove a task when its exit condition is met; detailed history belongs in commits
and sealed experiment artifacts.

### P0 - Trust what the system reports

- [ ] **Establish strict causal-control evidence.** Run a build-matched
  learned/frozen/random comparison whose V6 decision audits and semantically
  bound completion artifacts all validate. Done when learned updates cause
  valid same-state choice changes and both controls consume no update benefit.

- [ ] **Establish a strict reproducible baseline.** Repeat one- and two-worker
  versions of one predeclared workload under the same V6 audit/completion
  contract. Done when outcome, wall time, useful transitions/s, utilization,
  variance, and the largest reconciled loss are sealed and reproducible.

- [ ] **Make the learning chain auditable.** Trace observations, legal actions,
  choices, transitions, replay admission, updates, snapshots, and terminals;
  account for retries, rejections, duplicates, and censored work. Done when a
  sealed run explains every accepted and lost sample and broken links fail closed.

- [ ] **Decompose the critical runtime.** Separate observation, action,
  execution, replay, learning, publication, persistence, and reporting ownership.
  Done when dependency/source-size gates prevent new monoliths and each boundary
  has focused contract, replay, and fault tests.

### P1 - Make experimentation fast and operable

- [ ] **Remove measured bottlenecks.** Optimize the largest reconciled cost and
  retain matched before/after evidence; repeat until the ten-minute gate passes.

- [ ] **Prove save states pay for themselves.** Measure capture, restore,
  validation, fallback replay, branching yield, duplicates, memory, and useful
  work/state. Done when reuse materially beats replay from an authority point;
  replace or remove machinery that does not.

- [ ] **Make worker scaling real.** Remove measured IPC, serialization, queue,
  lock, checkpoint, and scheduling losses. Done when two workers repeatedly beat
  one on useful throughput with bounded resources and queues.

- [ ] **Make campaigns operable.** Add exact owned-process lifecycle, progress,
  ETA, cancellation, cleanup, crash recovery, and resume. Done when interruption
  tests leave no orphans or duplicate accepted experience and preserve results.

### P2 - Make the learner solve and optimize the route

- [ ] **Expose sufficient generic state and actions.** Version motion history,
  velocity, orientation, camera, analog input, prompt/action availability, and
  transition outcomes while retaining primitive controls. Done when ablations
  measure their value without hand-authored straight/wall/roll rewards.

- [ ] **Discover sparse terminals from scratch.** Supply enough native horizon
  and exploration diversity to reach a human-reachable load zone. Done when
  repeated held-out runs beat both controls without demonstrations.

- [ ] **Learn reusable multi-frame tactics.** Induce, parameterize, compose,
  promote, and retire tactics while primitives remain available. Done when they
  improve held-out sample efficiency without an authored Ordon sequence.

- [ ] **Improve after first success.** Propagate sparse terminal value, escape
  local optima, and optimize authenticated tick cost without stopping exploration
  or updates. Done when held-out runs pass the 123-tick cold-replay gate.

- [ ] **Validate optional demonstrations.** Treat human replay only as off-policy
  experience. Done when an ablation measures its sample-efficiency effect, it
  does not cap the learned policy, and scratch learning still succeeds without it.

### P3 - Harden and generalize

- [ ] **Make runs logically reproducible.** Define ordering and ownership for
  concurrent sampling, updates, publication, snapshots, and persistence. Done
  when fresh and resumed runs reproduce decision/evidence identities.

- [ ] **Harden durable state.** Keep checkpoints, replay, models, and manifests
  compact, bounded, binary, versioned, checksummed, and atomic. Done when
  migration tests accept supported versions and all invalid state fails closed.

- [ ] **Generalize without route changes.** Apply unchanged observation, action,
  learning, orchestration, and evaluation contracts to a second native route.
  Done when it passes scratch discovery, controlled improvement, and cold replay.

## Non-negotiable experiment rules

- The native load-zone predicate is the only route-success authority.
- Compare treatments by native experience and fixed resource budgets, not by a
  convenient number of decisions or favorable seeds.
- Every experiment answers one falsifiable question and records build, inputs,
  seeds, budgets, treatment, failures, and complete accounting.
- Throughput means unique useful native experience per wall-clock second. Raw
  attempts, duplicate branches, replayed frames, and queued work do not count.
- A faster benchmark is evidence only when the mechanism is route-independent
  and controls, ablations, and cold replay rule out benchmark gaming.
