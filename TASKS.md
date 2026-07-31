# Tasks: make the learning framework learn, quickly

This is the unfinished work queue. Delete tasks when they are complete; history
belongs in commits and benchmark bundles, not here.

## Goal

Given a restorable game state, observations, the actions legal in that state,
and a terminal predicate, discover and improve a route without an authored
route or benchmark-specific reward. Do it quickly enough to be useful on
larger problems.

The Ordon load zone is the first acceptance case. The human replay is `131`
native ticks. Reaching the real load-zone predicate from scratch proves basic
discovery; beating `131` proves optimization; `123` or lower is the first
credible route-quality result.

## Current blockers

- We have no retained current-build proof that learned ranking reaches the
  terminal more efficiently than frozen-ranking or random-valid controls.
- Fixed work is still slow: 256 useful expansions take about 115 seconds
  (`2.22/s`). About 58 seconds is tactic execution, 20 seconds persistence,
  and 9 seconds finalization.
- Shared replay content removed duplicate replay admissions (`512` to `256`),
  but did not improve total wall time. The bottleneck was displaced, not
  removed.
- We have no current proof of reliable scratch discovery, improvement after
  the first terminal, a route below `131`, or transfer to another problem.

## 1. Prove that the learner actually changes search

- [ ] Add tests that fail if terminal, censored, unsupported, or stale outcomes
      are trained with the wrong meaning; restart must reproduce the same
      learner state and rankings.
- [ ] Verify that each state exposes every legal primitive action and its
      parameters, including analog direction/duration, roll, camera modifier,
      and prompted actions. Availability is state, not a hidden scheduler rule.
- [ ] Verify that observations expose generic motion evidence needed to learn:
      position, velocity, recent trajectory and momentum change, camera state,
      contacts and their measured kinematic effect, action history, prompted
      actions, and terminal evidence. Missing values must be explicit.
- [ ] Run matched learned-ranking, frozen-ranking, and random-valid native
      cells with identical checkpoint, predicate, seeds, horizons, budgets,
      action schema, and worker topology.
- [ ] Show on development and held-out seeds that learning reduces unique
      useful expansions to the real terminal. If it does not, repair the
      learning target, features, exploration, or update cadence before tuning
      route quality.
- [ ] Treat an ordinary human replay as optional experience and run an
      ablation. It may improve sample efficiency, but must not become a route,
      waypoint source, incumbent policy, or requirement for success.

Done when learned ranking beats both controls in matched native search and the
improvement can be explained from retained state/action/outcome evidence.

## 2. Make exploration capable of finding the route

- [ ] Use horizons long enough for an unskilled policy to wander around the
      corner and reach the load zone; budget exhaustion is unknown, not
      terminal failure.
- [ ] Ensure workers explore distinct useful node/action expansions instead of
      repeating the same frontier, overcommitting to the greedy straight-line
      local optimum, or silently pruning unsupported actions.
- [ ] Retain per-seed time and useful expansions to first terminal, selection
      mode, coverage, uncertainty, retries, and stop reason.
- [ ] Reach the real load-zone predicate from scratch across four development
      seeds and held-out seeds without authored coordinates, route-shaped
      reward, seed mining, or a hand-authored action sequence.

Done when scratch discovery is repeatable, and failures can be classified as
learning, exploration, action/observation coverage, throughput, or execution
correctness rather than guessed at from a giant report.

## 3. Raise useful-evidence throughput

- [ ] Split tactic-execution time into checkpoint materialization,
      restore/capture, controller preparation, native simulation, IPC, and
      worker idle/queue time; optimize the largest measured component.
- [ ] Split finalization time into graph/replay compaction, checkpoint writes,
      validation, metrics, and report construction; remove work proportional
      to the full campaign history.
- [ ] Audit hot paths for repeated whole-graph, replay, route, checkpoint, or
      model cloning, hashing, serialization, verification, and durable flushes.
      Add growth tests for every repaired path.
- [ ] Make checkpoint reuse, frontier leasing, dispatch, and result admission
      scale without duplicate work, cache thrash, stale learners, or a serial
      coordinator bottleneck.
- [ ] Bound model-update cost and learner publication lag as replay grows.
- [ ] After the serial path is repaired, run fixed-work 1/2/4/8/16-worker
      curves and report useful expansions/second, parallel efficiency, worker
      occupancy, memory, cache behavior, retries, and learner staleness.
- [ ] Derive the required expansion rate from the measured
      expansions-to-terminal distribution and meet it with headroom for a
      five-minute median and fifteen-minute worst-seed discovery target.

Done when added workers produce useful independent evidence and measured
throughput makes the discovery target feasible.

## 4. Optimize successful routes

- [ ] Continue learning after the first terminal instead of ending the
      campaign or freezing the first successful path.
- [ ] Make successful paths branchable at intervals no coarser than four
      native ticks and evaluate counterfactuals from their interior states.
- [ ] Rank counterfactuals by predicted complete root-to-terminal outcome while
      retaining broad exploration; exact native terminal results remain truth.
- [ ] Demonstrate strict post-terminal improvement in at least three seeds.
- [ ] Beat the `131`-tick human replay, then reach `123` ticks or lower.
- [ ] Cold-replay the selected tape twice with the learner absent and require
      identical controller bytes, terminal tick/evidence, identities, and
      execution fidelity.

Done when a `123`-tick-or-lower route is learned and reproduces exactly.

## 5. Learn reusable tactics and transfer

- [ ] Discover parameterized action compositions from successful paths and
      counterfactuals; do not author blessed tactics in the UI or scheduler.
- [ ] Learn state-conditioned applicability from multiple independent native
      occurrences and retain the complete primitive sequence and realized
      transition chain.
- [ ] Promote a composition only when it beats its primitives on held-out
      states; keep all legal primitives selectable.
- [ ] Show that promoted tactics improve held-out search without reducing
      terminal reliability or useful expansions/second.
- [ ] Repeat discovery and optimization on a second native route without
      changing rewards or authoring route structure for it.

Done when at least one discovered tactic transfers and the same learner solves
a second route problem.

## 6. Keep the framework trustworthy and changeable

- [ ] Replace raw-report archaeology with a compact campaign summary covering
      terminal outcome, route ticks, useful expansions, learning/control
      comparison, phase timing, utilization, retries, and dominant failure.
- [ ] Split `native_tactic_route_runner/campaign.rs` by responsibility:
      decision execution, durable commit/recovery, and finalization/reporting.
- [ ] Add fault-injection coverage before dispatch, during execution, after
      native completion, after recovery commit, and after decision commit.
- [ ] Prove long campaigns have bounded checkpoint memory, persistence cost,
      model-update cost, and learner staleness.
- [ ] Keep operational state versioned, binary, content-addressed, and
      fail-closed on identity/schema/fidelity mismatch. JSON remains limited
      to small requests and exported reports.
- [ ] Keep process ownership exact: coordinators may stop only child handles
      they created; never use process-name or ancestry-wide killing.
- [ ] Keep production modules below the enforced size gate and organized by
      single responsibility; add clean-checkout validation for tests, schemas,
      evidence bundles, and replayability.

Done when another engineer can reproduce, explain, resume, profile, and modify
a campaign without reverse-engineering a monolith or trusting hidden state.

## Experiment discipline

- Run the smallest experiment that can answer one named question.
- Compare sample efficiency (useful expansions to outcome) and execution
  efficiency (useful expansions per second); neither substitutes for the other.
- Change one causal factor at a time and use matched controls.
- Keep detailed history in immutable benchmark bundles, not this file.
- Never encode desired Ordon behavior such as straightness, rolling, wall
  avoidance, camera alignment, or waypoints as reward. Surface facts and legal
  actions so the learner can discover their value.
