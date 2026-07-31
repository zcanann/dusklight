# Learning framework: remaining work

This file tracks only unfinished work needed to build a generic route learner
that learns reliably and fast enough to use. It is ordered: do not buy more
compute for a stage whose premise has not passed the cheaper stage above it.
Delete completed items; commits and retained benchmark bundles are the history.

`TASKS_ROUTE_PLANNER.md` is a separate product backlog.

## Acceptance

Starting from a restorable native state, observable facts, legal actions, and a
real terminal predicate, the framework must:

- discover a route without authored waypoints, route reward, or action script;
- beat frozen-ranking and random-valid controls on development and held-out
  seeds, with human replay optional rather than required;
- discover the Ordon load zone in a five-minute median and a fifteen-minute
  worst retained seed;
- improve after first success, beat the 131-tick human replay, and reach 123
  native ticks or lower;
- replay selected routes and resume interrupted campaigns exactly; and
- repeat the result on a second route without route-specific learning changes.

The Ordon times are acceptance tests, not reward terms. Do not reward authored
notions of a good Ordon route such as straightness, rolling, wall avoidance,
camera alignment, coordinates, or waypoints. Expose generic facts and legal
actions and let the learner value them.

## 1. Establish a valid learning problem

- [ ] Run identical-budget learned, frozen-ranking, and random-valid campaigns.
      Require learned ranking to reduce useful expansions to terminal on
      development and held-out seeds before doing route-quality tuning.
- [ ] Ablate ordinary suboptimal human replay. It may improve sample efficiency
      but may not supply route structure or be necessary for success.

## 2. Make scratch discovery reliable

- [ ] Fix exploration horizon and censoring so detours can be discovered and
      budget exhaustion is unknown, never failure.
- [ ] Make workers lease distinct useful state/action expansions without
      hiding uncertain actions or discarding unsuccessful evidence.
- [ ] Discover the real load-zone predicate from scratch on at least four
      retained development seeds plus held-out seeds. Report the full
      expansions-to-terminal distribution.

## 3. Make it fast

- [ ] Replace report archaeology with one compact campaign summary: outcome,
      route ticks, useful expansions, learned/control delta, expansions/second,
      phase timing, utilization, retries, learner lag, and dominant failure.
- [ ] Profile one fixed-work native campaign with non-overlapping time for
      restore/capture, controller preparation, simulation, IPC, scheduling,
      persistence, fitting/publication, finalization, and idle time.
- [ ] Remove measured bottlenecks and whole-history hot-path work, then prove
      bounded history scaling and fixed-work 1/2/4/8/16-worker scaling. Meet the
      five-minute median and fifteen-minute worst-seed discovery targets.

## 4. Learn better routes

- [ ] Continue after first success by branching successful trajectories at no
      more than four-native-tick intervals and valuing complete
      root-to-terminal outcomes while retaining exploration.
- [ ] Show strict post-terminal improvement on three seeds, then beat 131 ticks
      and reach 123 ticks or lower. Cold-play the selected tape twice with
      identical controller bytes, terminal evidence, identities, and fidelity.

## 5. Harden the framework

- [ ] Make campaigns bounded and exactly resumable with versioned binary,
      content-addressed state, fail-closed identity/schema/fidelity checks, and
      fault injection at dispatch, execution, commit, and publication.
- [ ] Split oversized modules by responsibility and enforce the size gate.
      Decision execution, learning, persistence/recovery, transport, and
      reporting must be independently testable and profileable.
- [ ] Make orchestration ownership and limits explicit: exact child handles,
      bounded CPU/memory, cancellation propagation, and no process-name or
      broad-ancestry discovery or termination.
- [ ] Add clean-checkout validation for tests, schemas, retained evidence,
      deterministic replay, interrupted recovery, and benchmark summaries.

## 6. Prove generality

- [ ] Learn and promote parameterized action compositions only when native
      held-out evidence improves search without harming reliability or
      throughput. Keep primitive sequences and rejected alternatives; do not
      author a blessed tactic set.
- [ ] Repeat scratch discovery and post-success optimization on a second native
      route with the same observation contract, reward model, and learner.

## Experiment rules

- One named question and the smallest useful matched comparison per experiment.
- Report sample efficiency and execution efficiency separately.
- A faster route is not learning unless learned ranking beats both controls.
- Keep detailed results in immutable benchmark bundles, not this file.
- JSON is limited to small requests and exported reports; durable hot-path
  state uses versioned binary serialization.
