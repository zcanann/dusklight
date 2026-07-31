# Learning framework

## Goal

Build a general learning system that can discover useful behavior from
restorable native states, observations, legal actions, transition outcomes,
and real terminal predicates -- and do it quickly enough to iterate on.

For the current Ordon proving ground, success means:

1. discover the load zone from scratch;
2. learn more reliably and sample-efficiently than frozen and random-valid
   controls;
3. reach the terminal within a practical wall-clock budget; and
4. continue optimizing past the 125-tick human replay to 123 ticks or fewer.

Human demonstrations may accelerate learning, but authored routes, waypoints,
route-shaped rewards, blessed tactic sequences, and proxy terminals may not be
required. Route-planner product work belongs in `TASKS_ROUTE_PLANNER.md`.

## Current truth

- The pinned human replay reaches the real Ordon load zone in 125 native ticks.
- The four request-sealed seeds are split before further evaluation:
  development uses `104729` and `155921`; untouched held-out evaluation uses
  `130363` and `181081`.
- Earlier scratch campaigns did not show useful learning. In the first matched
  development run after sealing policy authority, learned reached the real
  terminal at 262 authenticated ticks while frozen-policy and random-valid did
  not reach it in the same 32-decision budget. The model earned deployment
  authority at decision 16 and the terminal was discovered at decision 29.
  This is promising single-seed evidence, not yet a repeated development or
  held-out result.
- Whole-source-state held-out calibration gates every current use of the
  goal-reachability model. Untrusted predictions remain visible, while v39
  route reports and v4 campaign summaries reject unproven action or frontier
  policy deployment. Learned-versus-control results can now test learning
  rather than an authority leak.
- The immediate problem is framework correctness and learning quality -- not
  mining more seeds or treating a near miss as success.

## Work queue

Work in this order. After a failed gate, repair the first demonstrated cause
before running another large campaign.

- [ ] **Show that the system actually learns.** Run matched learned,
      frozen-policy, and random-valid treatments with identical seeds, action
      surfaces, horizons, workers, and resource budgets. Learned must improve
      terminal-discovery rate and time/sample-to-terminal on development seeds,
      then repeat that result on untouched held-out seeds. If it fails, inspect
      the causal chain and repair the first broken or uninformative link; do not
      compensate with route-specific rewards or seed mining.

- [ ] **Make learning fast enough to use.** Measure useful native transitions
      per second and attribute wall time to simulation/save states, transport,
      scheduling/idle capacity, learning, and persistence. Fix the largest
      measured bottleneck, repeat the same workload, and retain the before/after
      evidence. Scale workers only while throughput improves within explicit
      CPU and memory limits. Target scratch terminal discovery in a five-minute
      median and within fifteen minutes for every retained and held-out seed.

- [ ] **Harden the active framework and orchestration path.** Separate native
      execution, restoration, observations/actions, learning, graph search,
      persistence, orchestration, and reporting behind testable interfaces.
      Split oversized or mixed-responsibility modules and enforce a source-size
      regression gate. Campaigns must directly own child processes,
      cancellation, budgets, artifacts, and cleanup. Preserve deterministic
      replay/resume, treatment isolation, bounded binary durable state, schema
      migration, and clean-checkout reproduction. Add enough introspection to
      explain every learner decision, discarded sample, stalled worker, and
      throughput loss.

- [ ] **Prove post-terminal optimization.** Continue learning after first
      success, beat the 125-tick replay, and produce a route of 123 native ticks
      or fewer. Cold-replay the final controller twice with identical bytes,
      state identities, terminal evidence, and tick count.

- [ ] **Prove extensibility without route scripting.** Surface legal
      contextual actions such as roll, jump, mount, or interact when available,
      and let the learner compose parameterized primitives such as direction,
      camera lock, and action timing. Measure the optional human replay as
      experience: it may improve sample efficiency, but scratch learning must
      still work and the demonstration must not become a performance ceiling.
      Finally, repeat discovery and optimization on a second native route
      without changing learner logic or the observation, action, and objective
      contracts.

## Operating rules

- The terminal predicate is authoritative. Distance, speed, straightness,
  rolling, wall contact, and similar signals are observations or diagnostics,
  not hand-authored substitutes for success.
- Every experiment answers one question with the smallest matched run capable
  of answering it. Report terminal success, native ticks, samples, wall time,
  resource use, and uncertainty.
- A large campaign requires a named hypothesis or a verified throughput run.
  Repeating an unchanged failing campaign is not progress.
- Durable machine state is versioned, bounded, and binary. JSON is limited to
  small requests and human-facing reports.
- Remove completed work from this file. Commit and push each natural milestone;
  do not leave a long-lived dirty workspace.
