# Learning framework critical path

This file tracks only unfinished work needed to build a generic framework that
actually learns and does so fast enough to be useful. Completed work is removed;
git history and retained evidence bundles are the record. Route-planner product
work belongs in `TASKS_ROUTE_PLANNER.md`.

## What counts as success

The framework starts from a restorable native state and receives observations,
legal actions, and a real terminal predicate. Without authored routes,
waypoints, tactic sequences, or route-shaped rewards, it must:

1. beat matched frozen-policy and random-valid controls on development and
   held-out seeds;
2. find the Ordon terminal in a five-minute median and within fifteen minutes
   on every retained seed;
3. continue learning after first success, beat the pinned human replay, and
   produce a route of 123 native ticks or fewer with exact cold replay; and
4. repeat discovery and improvement on a second route without changing the
   learner, objective, or observation/action contract.

A lucky route, an unbounded mining campaign, or benchmark-specific shaping is
not evidence that the framework learns. Ordon coordinates, straightness,
rolling, wall contact, camera alignment, and human input are observations,
available actions, or optional experience - never hard-coded policy or reward.

## Work queue

Complete these gates in order. Supporting cleanup belongs in the gate it
unblocks, not in an unrelated backlog.

### P0 - Trust the framework

- [ ] Make the learning loop auditable end to end: observed state and legal
      actions -> exploration -> published experience -> learner update ->
      deployed policy -> changed behavior. Emit one compact campaign report
      with pinned inputs, treatment, outcome, samples/time to terminal,
      expansions/second, phase timings, utilization, learner lag, retries,
      rejected work, resource peaks, and evidence identities.
- [ ] Harden campaign ownership and clean-checkout validation. Each campaign
      must own exact child handles, cancellation, resource budgets, artifacts,
      and cleanup; validation must cover deterministic replay, treatment
      isolation, interruption/resume, schema compatibility, bounded history,
      and retained evidence. Never discover or terminate work by process name
      or broad ancestry.
- [ ] Enforce maintainable boundaries. Split oversized modules into native
      execution, learning/search, replay/graph, persistence/recovery, transport,
      orchestration, and reporting responsibilities; test each boundary and add
      a source-size gate. Keep durable machine state versioned, bounded, and
      binary, with JSON limited to small requests and human-facing reports.

### P1 - Prove real learning

- [ ] Run identical-budget learned, frozen-policy, and random-valid treatments
      on retained development and held-out seeds. Horizons must permit unguided
      terminal discovery. Report terminal rate, samples/time to terminal,
      useful expansions, and uncertainty across seeds.
- [ ] If learning does not beat both controls, use the causal audit to fix the
      first place experience is lost, duplicated, censored, stale, or ignored.
      Repeat the matched comparison until the advantage reproduces on held-out
      seeds; do not add route-shaped rewards or authored tactic sequences.
- [ ] Ablate the ordinary suboptimal human replay. Measure any sample-efficiency
      gain while proving replay is optional and the learned policy can surpass
      it.

### P2 - Make it fast enough to use

- [ ] Profile fixed work with non-overlapping time for native simulation/state,
      transport, scheduling, learning, persistence, finalization, and idle time.
      Report sample efficiency separately from execution efficiency.
- [ ] Remove measured dominant costs, then re-profile: duplicated exploration,
      serialization/copies, contention, whole-history work, policy lag, idle
      workers, and unbounded memory or persistence growth.
- [ ] Demonstrate useful scaling under explicit CPU/memory caps and choose the
      default worker count by time-to-result. Meet the five-minute median and
      fifteen-minute worst-seed discovery target on development and held-out
      seeds.

### P3 - Improve and generalize

- [ ] Continue learning after first terminal without collapsing exploration.
      Beat the pinned human replay, reach 123 native ticks or fewer, and cold
      replay the controller twice with identical bytes, terminal evidence,
      tick count, and identities.
- [ ] Let the learner discover parameterized and composed legal actions while
      retaining primitives and rejected evidence. Promote compositions only
      from held-out results, never from a blessed tactic list.
- [ ] Repeat scratch discovery and post-success improvement on a second native
      route without changing the learner, objective, or observation/action
      contract.

## Working rules

- Run the smallest matched experiment that answers one named question.
- Never substitute one fast route for evidence of learning.
- Do not optimize throughput before measuring where fixed work goes.
- Architecture changes must close a correctness hole, remove a measured
  bottleneck, or make the learning claim materially easier to audit.
- Commit and push each natural milestone; do not leave a long-term dirty
  workspace.
