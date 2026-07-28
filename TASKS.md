# Active tasks: make route learning auditable, scalable, and effective

This file contains only unfinished learning-framework work. Completed
implementation belongs in Git; benchmark history belongs in immutable reports.

## Product objective

Build a generic checkpointable optimizer that:

1. starts from an authenticated native checkpoint;
2. observes typed game state, action availability, and measured trajectories;
3. proposes primitive controller actions and learned tactic compositions;
4. evaluates alternatives from retained native states;
5. shares authenticated transitions with a state/action learner;
6. discovers and validates reusable tactics; and
7. minimizes native input ticks to an authenticated terminal.

The UI is not a TAS authoring surface. A human recording may be optional
experience, but it must not define privileged actions, observations, rewards,
state, or terminal semantics.

## Current benchmark truth

Ordon is the acceptance benchmark. The eligible request starts at authenticated
boundary `506`; the human incumbent first reaches the actual load-zone terminal
at tick `125`.

- The best machine result currently ties first-hit tick `125`. It does not beat
  the benchmark.
- Four lanes with four proposals each completed 128 decisions and 10,203
  evaluated native ticks in 202 seconds, yielding 373 useful transitions.
- One lane widened to sixteen proposals took 339 seconds for only 32 decisions.
  Wider sibling batches are not a substitute for learner iterations.
- The native checkpoint primitive is functional and authenticated. The latest
  four-lane run reported a 100% cache-hit rate and about 27.5 ms mean restore,
  but multi-lane orchestration still spends too much time rebuilding and
  persisting frontiers.
- A four-lane run is one generation. Its lanes share the input corpus but not
  each other's new experience; only a later generation consumes the merged
  results. One-generation results must not be described as online shared
  learning.

A terminal hit, a reliable terminal hit, a 125 tie, or a faster search for the
same tie is diagnostic evidence only.

## Non-negotiable boundaries

- Production modules have one reason to change. Organize related behavior in
  named module folders; do not use grab-bag `utils`, `common`, or `misc`
  modules.
- A production Rust source file must stay below 1,500 physical lines, with a
  normal target below 1,000. Test modules belong in adjacent test files once
  they materially obscure production behavior. Existing oversized files are
  debt to split, not precedent for raising the limit or adding more code.
- CI must reject new oversized files and any growth in a grandfathered
  oversized file. Every cleanup milestone lowers the grandfathered ceiling
  until no exception remains.
- Prefer typed collaborators with narrow ownership over functions that accept
  sprawling configuration/state argument lists. A module boundary must reflect
  responsibility and data ownership, not merely move lines behind `include!`.
- Reward is authenticated terminal value minus native input cost. Trajectory,
  velocity, collision, straightness, rolling, and prompted-action availability
  are observations or auxiliary prediction targets, not handcrafted utility.
- JSON is not an operational checkpoint, replay, transition, model, frontier,
  journal, or learned-tactic format. Small authored requests and exported
  reports may use JSON.
- Every evaluated native transition is eligible learning evidence. Every exact
  terminal candidate is retained independently of whether the policy selected
  it. Evaluation results must never retroactively replace the policy action.
- Human experience is ordinary, ablatable replay. The system must retain a
  from-scratch lane and must be able to improve beyond the demonstration.
- Performance reports include process launch, native simulation, restore,
  checkpoint capture, learner update, orchestration, persistence, and evidence
  projection. Do not move overhead outside the measured boundary.
- First-hit comparisons must bind the same source checkpoint, terminal
  predicate, game bytes, card fixture, fidelity, and source boundary.

## P1 - Build a real shared replay and learner control plane

- [ ] Add one append-only binary replay/frontier service per campaign. Workers
      publish authenticated transitions; they do not own private authoritative
      corpora.
- [ ] Deduplicate by exact transition identity while preserving distinct input
      lineages that reach similar observed states.
- [ ] Publish immutable, versioned learner snapshots and bind every decision to
      the exact snapshot used.
- [ ] Support two explicit execution modes:
  - deterministic generation barriers for reproducible comparisons; and
  - bounded-staleness asynchronous updates for throughput.
- [ ] In asynchronous mode, make newly admitted transitions and terminal
      candidates visible without waiting for an entire four-lane generation.
- [ ] Measure replay admission latency, learner updates per second, snapshot
      staleness, useful transitions per update, and duplicate/censored rows.
- [ ] Add interruption tests proving that replay, frontier, learner, and
      candidate identities resume without lost or repeated authority.

Acceptance:

- A second lane can learn from a first lane's admitted transition at the next
  declared sharing boundary.
- Scaling worker count increases useful learner updates instead of merely
  multiplying independent searches.

## P2 - Make native checkpointing buy throughput

- [ ] Benchmark, separately, process launch, authenticated-root replay,
      process-local restore, host snapshot transfer, fact extraction, and
      checkpoint capture at representative early, middle, and late frontiers.
- [ ] Keep process-local checkpoint handles owned by persistent workers and
      route follow-up jobs to the owning worker.
- [ ] Remove the current coupling that disables cross-decision direct restore
      merely because a campaign has multiple seeds.
- [ ] Add a bounded checkpoint residency policy with explicit byte accounting,
      eviction, replay fallback, and no hidden unbounded emulator copies.
- [ ] Preserve exact portable replay reconstruction for evicted or
      process-lost checkpoints.
- [ ] Audit headless execution to prove which renderer, audio, pacing, and
      presentation systems still run. Disable only work whose removal preserves
      native state and terminal parity, then measure the result.
- [ ] Report direct-restore rate and useful transitions per restore, per native
      simulation second, and per wall second.

Acceptance:

- After warm-up, ordinary non-root expansions use direct restore unless an
  explicitly reported ownership, eviction, or process-loss condition prevents
  it.
- The same transition and terminal evidence are byte-identical through direct
  restore and authenticated replay fallback.
- Orchestration plus persistence no longer dominates native simulation on the
  fixed throughput benchmark.

## P3 - Prove that the learner solves delayed continuous-control credit

- [ ] Compare the current local generalized model against at least:
  - fitted Q over a learned continuous representation;
  - a double-Q or ensemble control;
  - a conservative offline control; and
  - a non-learning structured-search baseline.
- [ ] Calibrate value and uncertainty on held-out state regions and held-out
      action realizations, not random rows from the same correlated route.
- [ ] Run matched demonstration-assisted and from-scratch ablations. The
      demonstration may improve sample efficiency but may not cap the policy at
      the demonstrated route.

Acceptance:

- Learned return is the only action-utility ordering.
- Auxiliary signals improve representation or prediction without becoming
  reward shaping.
- The learner reliably escapes the around-corner local optimum and improves a
  suboptimal demonstrated route.

## P4 - Discover and reuse tactics instead of blessing a fixed list

- [ ] Keep primitive actions generic and state-local: analog direction,
      duration, camera modifier, and currently available prompted buttons.
- [ ] Mine repeated successful action subsequences and parameter relationships
      from authenticated replay.
- [ ] Promote a tactic only after it improves terminal/tick return on held-out
      source states relative to its primitive components.
- [ ] Represent promoted tactics with typed entry conditions, bounded execution,
      emitted controller input, outcome distributions, and exact lineage.
- [ ] Allow primitive and promoted tactics to compete under the same learner;
      promotion must not permanently remove primitive exploration.
- [ ] Measure whether promotion improves useful transitions per wall second and
      time-to-best-route on held-out seeds.

Acceptance:

- Useful compositions can be discovered without hard-coding an Ordon route.
- A promoted tactic reproduces exactly and provides measurable held-out search
  value.

## P5 - Establish the capacity curve before buying 10x or 100x volume

- [ ] Run fixed-plan scaling trials at 1, 2, 4, 8, and 16 workers after P0-P2.
- [ ] Separate proposal parallelism, environment parallelism, and learner-update
      parallelism in the report.
- [ ] Plot useful transitions, learner updates, unique frontier cells, native
      ticks, restore traffic, and best first-hit tick against wall time and
      memory.
- [ ] Identify the saturation point and the responsible resource: native
      simulation, restore bandwidth, learner fit, persistence, scheduling, or
      duplicated exploration.
- [ ] Demonstrate at least a 10x improvement in time-to-fixed-evidence or explain
      with measured scaling limits why additional hardware cannot provide it.
- [ ] Do not plan 100x capacity until the 10x curve shows useful near-linear
      scaling.

## P6 - Beat and verify the authenticated Ordon incumbent

- [ ] Produce a machine-generated candidate whose first authenticated load-zone
      hit is strictly earlier than tick `125` from boundary `506`.
- [ ] Continue to a credibility target of tick `123` or lower; a tick-124 result
      clears the formal regression gate but is not strong evidence that the
      framework is ready for harder routes.
- [ ] Minimize the complete successful controller tape without changing source,
      terminal, game build, fixture, or execution fidelity.
- [ ] Cold-replay the complete minimized tape at least twice from process boot
      with the learner and tactic executors out of the loop.
- [ ] Require byte-identical input and identical authenticated terminal evidence
      across cold replays.
- [ ] Publish the execution plan, learner/replay lineage, complete timing,
      worker topology, peak memory, winning route lineage, first-hit tick, and
      proof identities.

Anything short of the sub-125 cold-replay proof is progress evidence, not task
completion.
