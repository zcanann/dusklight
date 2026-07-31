# Learning framework

## Purpose

Build a general system that can discover, learn, and optimize routes from:

- restorable native states;
- observations and legal actions;
- measurable transition outcomes; and
- an actual terminal predicate (for Ordon, entering the load zone).

The learner may use optional human experience, but it must not require an
authored route, waypoints, route-shaped rewards, or a blessed tactic sequence.
This file contains only unfinished work needed to make that system learn and
return useful results quickly. Route-planner product work belongs in
`TASKS_ROUTE_PLANNER.md`.

## Current baseline

- The pinned human replay reaches the Ordon terminal in 125 native ticks.
- A valid matched 16-decision experiment did not reach the terminal. Learned
  policy was also slower than frozen-policy and random-valid controls.
- The first diagnosed failure was premature deployment of a small,
  poorly-calibrated goal-reachability model. Whole-source-state held-out
  calibration now keeps that model as visible sibling evidence until it proves
  better-than-chance ranking. The next matched experiment must determine
  whether policy updates now improve behavior.

## Ordered work

Work top to bottom. If a result fails, add only the task required to repair the
first demonstrated failure, then repeat that gate.

- [ ] **Prove that policy updates improve behavior.** Run learned,
      frozen-policy, and random-valid treatments with identical seeds, action
      surfaces, horizons, worker counts, and resource budgets. Use horizons
      long enough for unguided terminal discovery. Learned must beat both
      controls in terminal success and time/sample-to-terminal on retained
      development seeds, then on untouched held-out seeds. If it does not,
      audit observation -> proposal -> outcome -> credit -> update -> next
      decision and repair the first broken or uninformative link.

- [ ] **Make discovery fast enough to use.** Measure wall time and useful
      transitions separately. Attribute wall time to native simulation and
      save-state work, transport, scheduling, learning, persistence, and idle
      capacity. Fix measured bottlenecks and verify scaling under explicit CPU
      and memory limits. Scratch learning must discover the Ordon terminal in
      a five-minute median and within fifteen minutes for every retained and
      held-out seed.

- [ ] **Prove post-terminal optimization.** Continue learning after first
      success, surpass the 125-tick human replay, and produce a route of 123
      native ticks or fewer. Cold-replay the final controller twice with
      identical bytes, identities, terminal evidence, and tick count.

- [ ] **Prove demonstrations help without becoming a ceiling.** Treat the
      human replay as optional experience. Measure its effect on sample
      efficiency while retaining scratch success and the ability to surpass
      the demonstration.

- [ ] **Prove the action system can grow without route scripting.** Expose
      legal prompted actions when available and support learner-selected,
      parameterized compositions (including direction/camera-lock/roll timing)
      without removing primitives. Promote a composition only when matched
      held-out evidence shows an improvement.

- [ ] **Prove generality.** Repeat scratch discovery and post-terminal
      improvement on a second native route without changing learner logic or
      the observation, action, and objective contracts.

## Framework invariants

These are required continuously. Repair a violation when it blocks or
invalidates an item above; do not create speculative cleanup campaigns.

- Experiments answer one named question with the smallest matched run that can
  answer it. Report terminal success, native ticks, samples, wall time, resource
  use, and uncertainty; never substitute proxy movement for the terminal.
- Every decision is auditable from observation through legal actions,
  prediction/selection, native outcome, credit assignment, model update, and
  the next consumed policy snapshot. Uncertainty and rejected evidence remain
  visible.
- Campaigns directly own child handles, cancellation, budgets, artifacts, and
  cleanup. They never discover or terminate work by process name or broad
  ancestry, and they never exceed configured CPU or memory limits.
- Resume, deterministic replay, treatment isolation, bounded state/history,
  schema migration, and clean-checkout reproduction are tested.
- Durable machine state is versioned, bounded, and binary. JSON is reserved for
  small requests and human-facing reports.
- Native execution, learning, replay/graph, persistence, transport,
  orchestration, and reporting have testable single responsibilities.
  Oversized modules are split, and an automated source-size gate prevents
  regression.
- Completed work is removed from this file. Commit and push every natural
  milestone; do not leave a long-lived dirty workspace.
