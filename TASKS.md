# Active task: build a route optimizer that beats the incumbent

This file contains only unfinished learning-framework work. Completed work and
implementation history belong in Git and benchmark reports.

## Objective

Build a generic checkpointable search-and-learning system that:

1. starts from an authenticated native checkpoint;
2. observes typed game state and derived progress measurements;
3. proposes controller tactics and bounded tactic compositions;
4. evaluates many alternatives from retained states;
5. learns which state/action transitions are valuable;
6. discovers and promotes reusable tactics; and
7. optimizes a terminal-reaching route for the fewest native input ticks.

Ordon is the current acceptance benchmark. Its authenticated incumbent first
reaches the terminal at tick 125. A terminal reach, a reliable terminal reach,
or a 125-tick tie is diagnostic evidence only. The framework has not succeeded
on this benchmark until a machine-generated route first reaches the same
terminal in fewer than 125 ticks and reproduces from cold boot.

The browser workbench records, inspects, and replays execution graphs. It is not
a TAS authoring surface. Human recordings may supply optional experience but
must not define privileged actions, observations, state, or terminals.

## Non-negotiable architecture

- JSON is not an operational checkpoint, replay, transition, model, frontier,
  or journal format.
- Reuse the existing versioned binary envelopes, tape encoding, native episode
  shards, transition corpus, content digests, and zstd support.
- Keep serialization behind storage interfaces. In-memory learner types must
  not depend on JSON or a particular file layout.
- Small authored requests and exported human-readable reports may use JSON.
- Hot exploration records only the data required to resume, learn, and verify
  identities. Full evidence graphs and readable reports are projections
  produced after a candidate is retained.
- Every performance claim must include useful native simulation, restore,
  orchestration, and persistence time. Do not hide overhead outside the
  measured boundary.

## P1 - Use the simulator's available throughput

Acceptance:

- A multi-worker tactic-route campaign demonstrably uses the requested workers.
- End-to-end learning throughput improves by at least an order of magnitude on
  the sealed Ordon workload without changing its semantic results.
- Worker scaling is measured rather than inferred from raw simulator tests.

## P3 - Replace the fixed action grid with tactic proposals

- [ ] Generate multiple diverse parameter proposals from each retained state
      using progress, novelty, uncertainty, and previous outcomes.

Acceptance:

- The learner can evaluate parameters and compositions that were not present as
  individual blessed catalog entries at campaign start.
- The same generic tactic runtime works for Ordon and at least one held-out
  start/goal pair without route-specific preferred-action scripts.

## P4 - Discover, validate, and promote tactics

- [ ] Mine successful and high-value replay fragments for recurring bounded
      action sequences and state-conditioned tactic parameters.
- [ ] Propose reusable macro tactics from those fragments without modifying the
      underlying controller-input contract.
- [ ] Evaluate each candidate macro against its primitive components from
      multiple authenticated frontier states and deterministic seeds.
- [ ] Promote a tactic only when it improves terminal probability, progress per
      simulated tick, or route cost under a sealed comparison.
- [ ] Retain provenance from source transitions through composition, evaluation,
      promotion, and every later execution.
- [ ] Demote tactics that cease to add value while keeping historical replay
      data readable.

Acceptance:

- At least one tactic absent from the initial catalog is discovered, promoted
  by measured results, reused in a later decision, and cold-replayed exactly.
- Promotion is based on comparative execution evidence, not manual blessing.

## P5 - Couple search and learning around useful trials

- [ ] Use the value/Q model to rank batches of state-conditioned tactic
      proposals and frontier states rather than blocking on one epsilon-greedy
      categorical action at a time.
- [ ] Preserve a diverse frontier across progress, novelty, uncertainty, route
      cost, and terminal evidence so one choke state cannot absorb the campaign.
- [ ] Prioritize transitions that reduce uncertainty or cross poorly covered
      state boundaries, including the shared Ordon choke observed by failed
      seeds.
- [ ] Train from the deduplicated replay store and measure model-update cost
      separately from simulation and persistence.
- [ ] Compare the learned ranking against random valid proposals and
      non-learning structured search under identical native-tick budgets.
- [ ] Publish failures, coverage, candidate diversity, and time to improvement;
      do not substitute terminal success rate for route optimization.

Acceptance:

- Learning improves sub-incumbent discovery rate or best route cost over both
  equal-budget baselines across multiple seeds.
- Added simulation volume produces distinct useful transitions rather than
  repeated trajectories from one parent checkpoint.

## P6 - Beat the authenticated Ordon incumbent

- [ ] Run the integrated system from the exact `to_ordon_spring_q125` source
      boundary with no route-specific preferred-action script.
- [ ] Search until it produces a candidate whose first authenticated terminal
      hit is strictly earlier than tick 125.
- [ ] Minimize the complete successful controller tape without changing the
      source checkpoint, terminal predicate, game build, or execution fidelity.
- [ ] Cold-replay the complete winning tape at least twice from process boot
      with the learner and controller out of the loop.
- [ ] Require byte-identical input and identical authenticated terminal
      evidence across the cold replays.
- [ ] Publish total native simulation, wall time, worker count, peak memory,
      winning lineage, first-hit tick, and proof identities.

Acceptance:

- A machine-generated route reaches the authenticated Ordon terminal in fewer
  than 125 ticks.
- The route reproduces exactly from cold boot.

Anything short of both conditions is progress evidence, not success.
