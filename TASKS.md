# Route-learning framework

## Mission

Build an algorithm-agnostic learning and search framework that can discover and
optimize native routes substantially beyond human play. Q-learning is one
candidate, not a requirement; use the method that learns best from the samples
and native throughput we can actually sustain.

The framework receives a native starting state, generic observations, the legal
action surface, and an authoritative terminal predicate. It must explore, learn
which behavior produced useful outcomes, turn discoveries into retained
policies or trajectories, minimize native completion cost, and emit an exactly
replayable input graph.

The action hierarchy must include all three sources:

- primitive controller inputs;
- reusable parameterized tactics supplied by us;
- tactics discovered, evaluated, promoted, composed, and retired by the system.

## ToOrdonSprings benchmark contract

Ordon is the first proving ground. The native load-zone predicate is the only
success authority, and native elapsed ticks are the score.

The headline result is **zero-shot**: the scored campaign starts from the named
root state without a human demonstration, authored route, waypoint sequence,
route-specific reward, proxy terminal, or tactic extracted from the human route.
It may use generic observations and a route-agnostic action/tactic library fixed
before the scored run. Human play may be evaluated later as optional off-policy
experience, but that is a separate treatment and cannot satisfy the zero-shot
claim.

The known human ToOrdonSprings route takes 125 ticks. Use this explicit ladder:

- **124 ticks:** first meaningful zero-shot human beat;
- **123 ticks:** stronger evidence that the framework can optimize beyond the
  baseline;
- **120 ticks or less:** clearly superior, TAS-grade headline result.

A score counts only when the framework's retained output reaches the native
load zone and cold-replays at least twice from the named root with identical
inputs, state identities, terminal proof, and tick count. A terminal reached by
an unselected coverage proposal is discovery evidence, not a scored route or
learning result.

## Work queue

These are the current missing capabilities, in execution order. Keep a thin
end-to-end campaign runnable, remove completed work from this file, and retain
detailed history in commits and sealed campaign artifacts.

### 1. Optimize selected zero-shot terminals

- [ ] Improve the retained 231-tick zero-shot route to 124 ticks or less using
  generic optimization. The first bounded two-worker residual campaign reduced
  its 239-tick handoff to 231; deterministic minimization reduced input
  complexity from 194 to 192, and two separately backed exact native replays
  reproduced the load-zone terminal at tick 231. This proves the handoff,
  reduction, and proof path works, but is not competitive route quality.
- [ ] Whenever optimization admits a strict incumbent improvement, trigger
  bounded minimization, splice the minimized suffix into the complete route,
  and cold-replay that route twice from the named root. A lucky discovery, an
  operator-run reducer, or checkpoint-local suffix proof is not a scored route.

Exit: each strict zero-shot improvement is automatically converted into a
selected, independently replayable route, and the retained route reaches the
authenticated terminal in 124 ticks or less.

### 2. Prove that accumulated experience improves future behavior

- [ ] Evaluate candidate learning/search algorithms against the real horizon,
  branching factor, transition volume, and update latency; do not preserve
  Q-learning by default.
- [ ] Learn from successful and unsuccessful trajectories using sparse terminal
  value, authenticated tick cost, and generic state deltas.
- [ ] Support online collection, off-policy replay, prioritized reuse, temporal
  credit assignment, continued exploration, and escape from local optima.
- [ ] Surface motion history, velocity, orientation, camera, analog input,
  prompted-action availability, action duration, and resulting state deltas.
  Straight travel, velocity loss, collision slowdown, rolling, camera alignment,
  and detours must be learnable rather than encoded as Ordon-specific rewards.
- [ ] Run learned/adaptive, frozen-policy, and random-valid treatments over the
  same candidate stream, native budget, and campaign design.
- [ ] Require the adaptive treatment to reproduce terminals more often, consume
  fewer native samples, or achieve lower terminal ticks on held-out seeds.
- [ ] Verify that useful behavior survives ordinary seed ordering, campaign
  composition, and update-cadence changes.

Exit: retained experience causally changes future choices and improves terminal
outcomes relative to both controls. Merely finding another coverage terminal
does not pass.

### 3. Beat and then substantially exceed the human route zero-shot

- [ ] Produce and independently verify a zero-shot 124-tick route.
- [ ] Continue the same generic learning/search process to 123 ticks or less.
- [ ] Reach 120 ticks or less without adding an Ordon-specific route, reward,
  waypoint, exception, or hand-authored input sequence.
- [ ] Minimize the winning input graph without changing its terminal identity or
  native tick count.
- [ ] Retain a fixed-budget result distribution, not only the best lucky seed,
  so sample cost and reliability remain visible.
- [ ] After the zero-shot result exists, run a separate human-demonstration
  ablation to determine whether ordinary human play improves sample efficiency
  without becoming required or imposing a policy ceiling.

Checkpoint: 124 ticks is the first human beat; 123 ticks is stronger progress.

Exit: a zero-shot route reaches the native load zone in 120 ticks or less and
cold-replays twice with identical evidence.

### 4. Learn and compose tactics

- [ ] Represent primitives and tactics through one interruptible, parameterized
  option interface so either can be chosen at every valid boundary.
- [ ] Supply route-agnostic tactics such as moving toward a chosen heading,
  one-frame direction plus camera lock, camera lock plus action, rolling, and
  state-conditioned analog steering.
- [ ] Keep every currently legal primitive and prompted action available,
  including roll, jump, mount, lift, and future game-specific affordances.
- [ ] Mine useful sub-trajectories and repeated control structure; propose
  tactics with learned initiation conditions, parameters, termination
  conditions, and expected outcomes.
- [ ] Promote tactics only when held-out results improve over primitives and
  existing tactics; retire regressions and permit tactic composition.
- [ ] Keep primitive actions available so a bad abstraction cannot cap the
  policy.

Exit: supplied tactics improve search without encoding an Ordon route, at least
one automatically discovered tactic improves held-out performance, and tactic
composition exceeds the trajectories from which it was learned.

### 5. Scale throughput only when the learning experiment demands it

The present campaign speed is sufficient to expose the missing learning and
terminal-optimization behavior. Do not continue golfing infrastructure merely
because a measurable subphase is nonzero.

- [ ] Re-profile only after terminal-aware rollouts are working, using existing
  raw evidence before launching another campaign.
- [ ] Optimize the largest measured cost only when it prevents useful learning
  iterations or pushes a standard two-worker campaign beyond ten minutes.
- [ ] Measure whether retained save-state branching beats replay from its
  authority point; simplify or remove it where it does not.
- [ ] Revisit deterministic generation-barrier or other concurrent learning
  schedules only after policy adoption and terminal optimization have causal
  tests.
- [ ] Eliminate measured worker starvation, redundant native work, unnecessary
  serialization, and central bottlenecks while keeping queues and memory
  bounded.

Exit: the next learning experiment, not an isolated microbenchmark, completes
within ten minutes; two workers materially improve unique accepted native
experience per second; retained save states have measured positive return.

### 6. Prove the framework is durable and general

- [ ] Apply the unchanged observation, action, learning, tactic, orchestration,
  and evaluation contracts to a second native route.
- [ ] Make fresh and resumed concurrent campaigns logically reproducible, with
  exact ownership for sampling, updates, model publication, checkpoints, and
  accepted experience.
- [ ] Keep models, replay, checkpoints, and manifests compact, bounded, binary,
  versioned, checksummed, atomic, and migration-tested.
- [ ] Refactor oversized and mixed-responsibility code along observation,
  execution, state, replay, learning, tactics, workers, persistence, and
  reporting boundaries; enforce source-size and dependency gates.

Exit: the second route passes zero-shot discovery, learned improvement over
controls, tactic use, and exact cold replay without route-specific framework
changes; interruption and resume reproduce accepted-experience identities.

## Engineering rules

These apply while completing every queue item; they are not a later cleanup
phase.

- Campaigns collect one comprehensive reusable evidence stream. A run should
  answer many questions, including useful analyses not anticipated when it was
  launched. Rerun only for a changed treatment, genuine replication, or raw
  evidence that could not have been captured previously.
- Search retained benchmark artifacts and commit history before implementing a
  treatment; repeat a rejected intervention only when a named premise changed.
- Record observations, legal actions, choices, executions, transitions, replay
  admissions, updates, tactic decisions, terminals, timings, retries,
  rejections, duplicates, and censored work with stable identities.
- Throughput means unique accepted native experience per wall-clock second, not
  attempts, queued work, duplicate branches, or replayed frames.
- Score retained outputs, not the luckiest proposal observed anywhere in a
  batch. Report discovery, policy adoption, optimization, and final replay as
  separate outcomes.
- Own child processes directly. Cancellation, timeout, crash recovery, cleanup,
  progress, and resume must not scan for or affect unrelated processes.
- Use focused tests during iteration and broad suites at meaningful integration
  checkpoints. Do not substitute repeated testing for implementation progress.
- Benchmark gains count only when generic mechanisms, held-out evaluation,
  exact replay, and appropriate controls rule out route-specific gaming.
