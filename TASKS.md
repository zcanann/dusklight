# Learning framework

This file tracks only unfinished work required to build a generic learning
framework that **actually learns** and does so **fast enough to be useful**.
Completed work is removed. Product work belongs in `TASKS_ROUTE_PLANNER.md`.

## Definition of done

Starting from a restorable native state, the framework receives observations,
legal actions, and a terminal predicate. Without an authored route, waypoint
sequence, route-shaped reward, or blessed tactic sequence, it must:

1. learn materially faster than matched frozen-policy and random-valid controls;
2. discover the Ordon terminal in a five-minute median and within fifteen
   minutes on every retained development and held-out seed;
3. continue improving after discovery, beat the pinned 125-tick human replay,
   and produce a route of 123 native ticks or fewer; and
4. repeat discovery and improvement on a second route without route-specific
   changes to the learner or observation/action/objective contracts.

Every result must cold-replay exactly. A lucky route, an unbounded search, or a
benchmark-specific heuristic is not evidence of learning.

## Current work

Complete these gates in order. Add work only when it is necessary to pass one
of them.

### 1. Make the learning claim trustworthy

- [ ] Run learned, frozen-policy, and random-valid treatments with identical
      budgets on retained development and held-out seeds. Horizons must be long
      enough for unguided discovery. If learned does not beat both controls,
      use the causal summary to find and fix the first broken link before doing
      more route mining.

### 2. Make time-to-result useful

- [ ] Measure sample efficiency separately from execution efficiency. Account
      for all campaign time across native simulation/state work, transport,
      scheduling, learning, persistence/finalization, and idle capacity.
- [ ] Fix the measured dominant constraints, then repeat the matched experiment.
      Test worker scaling under explicit CPU and memory caps; address duplicated
      work, policy lag, contention, serialization/copying, idle workers, and
      unbounded state only when evidence shows they matter. Meet the five-minute
      median and fifteen-minute worst-seed discovery target.

### 3. Prove useful learning, not benchmark gaming

- [ ] Continue learning after the first terminal. Beat the pinned human replay,
      reach 123 native ticks or fewer, and cold-replay the learned controller
      twice with identical bytes, terminal evidence, tick count, and identities.
- [ ] Treat human replay as optional experience: measure whether it improves
      sample efficiency while proving that scratch learning works and can
      surpass the demonstration.
- [ ] Let the learner discover useful parameterized or composed actions from the
      generic legal action surface. Retain primitives and rejected evidence;
      promote compositions only when held-out results improve.
- [ ] Repeat scratch discovery and post-terminal improvement on a second native
      route without changing the learner, objective, or observation/action
      contracts.

## Required framework quality

These are acceptance criteria for every gate, not a separate backlog:

- Campaigns own exact child handles, cancellation, budgets, artifacts, and
  cleanup. Never find or terminate work by process name or broad ancestry.
- Interruption/resume, deterministic replay, treatment isolation, schema
  compatibility, bounded history, and clean-checkout reproduction are tested.
- Native execution, learning/search, replay/graph, persistence/recovery,
  transport, orchestration, and reporting have testable single-purpose
  boundaries. Oversized modules are split and prevented by a source-size gate.
- Durable machine state is versioned, bounded, and binary. JSON is limited to
  small requests and human-facing reports.
- Every experiment answers one named question with the smallest matched run
  that can answer it. Commit and push each natural milestone.
