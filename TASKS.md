# Active tasks: build a learning framework that learns fast enough to use

This file is the forward work queue. It contains unfinished work only.

When a task is complete, remove it. Git and retained benchmark bundles are the
history. Do not append commit logs, experiment diaries, implementation
postmortems, or completed checklists here.

## Mission

Build a generic system that can learn efficient controller routes through a
deterministic game from an authenticated native checkpoint.

The framework receives:

- exact restorable game states;
- typed observations and currently applicable actions;
- controller inputs and their realized trajectories;
- a terminal predicate; and
- native ticks consumed.

It must:

1. discover a route to the real terminal predicate without an authored route;
2. improve that route using evidence gathered from native execution;
3. learn reusable state-conditioned actions and action compositions;
4. make enough useful attempts per second for learning to finish on practical
   time scales; and
5. remain auditable, recoverable, and maintainable while doing so.

The Ordon load-zone problem is the first acceptance case because it is
deliberately elementary. The ordinary human replay reaches the load zone at
tick `131`. Reaching it reliably is the minimum discovery gate. Beating `131`
shows optimization; tick `123` or lower is the first credible route-quality
result. If the framework cannot do this in minutes rather than hours, it is not
ready for harder problems.

## Current position

- A historical scratch campaign reached the load-zone predicate in four seeds,
  but its best reported route was `231` ticks and time to first terminal was
  roughly twelve minutes. Its complete per-seed evidence is unavailable, so it
  is directional evidence rather than the current baseline.
- The deterministic around-the-corner fixture shows that learned ranking can
  reduce expansions in a synthetic environment. There is still no retained
  native proof that learning beats matched scheduler-only and random-valid
  controls.
- The latest retained two-worker fixed-work campaign performs 256 useful
  expansions in about `113.3` seconds (`2.26` expansions/second). Roughly
  `57.8` seconds are tactic execution, `19.2` seconds are persistence, and
  `5.9` seconds are model update.
- Replay admissions now batch one durable journal flush per decision instead
  of one per proposal, but this change has not yet been measured in a matched
  native run.
- There is no retained current-build proof of reliable minutes-scale scratch
  discovery, repeated post-terminal improvement, a route below `131`, or a
  route at or below `123`.

These facts make the immediate order of work:

1. measure the already-implemented replay-batching treatment once;
2. establish an honest current native baseline;
3. prove whether learned ranking improves sample efficiency;
4. raise useful-evidence throughput to the rate required by that sample count;
5. make discovery and post-terminal optimization pass the Ordon gates; and
6. prove the same machinery transfers to reusable tactics and another route.

## P0 - Establish a decisive native baseline

- [ ] Run one current-build, single-seed scratch diagnostic with a horizon long
      enough for an unskilled policy to wander to the load zone. Do not use the
      human replay, authored coordinates, Ordon-specific rewards, or a
      hand-authored action sequence.
- [ ] Retain a self-contained evidence bundle for that run: request, execution
      binding, terminal predicate, game/controller identities, plan, seeds,
      per-decision trace, graph checkpoint, replay/model authority, resource
      audit, and any terminal tape/result.
- [ ] Make the audit answer, without manually reading a multi-megabyte report:
      - whether and when the real terminal predicate was reached;
      - useful expansions and wall time to first and best terminal;
      - why each node and action was selected;
      - which learner revision and applicable-action surface were consumed;
      - exploration versus learned selections;
      - restore, native simulation, IPC, learner, graph, and persistence time;
      - worker utilization, retries, fallbacks, and stop reason; and
      - first-route and best-route native ticks.
- [ ] Run matched learned-ranking, scheduler-only, and random-valid cells. Hold
      checkpoint, predicate, action schema, seed, horizon, proposal width,
      workers, expansion budget, wall budget, and fidelity fixed.
- [ ] Compare any discovered route with the `131`-tick replay using trajectory,
      velocity retention, action availability, roll/camera use, option
      duration, detour, contact-correlated slowdown, and repeated or idle work.
      This diagnoses missing state or actions; it does not authorize imitation
      reward or an authored route.
- [ ] From the observed expansions-to-terminal distribution, compute the
      minimum useful-expansion throughput required for a five-minute median
      and fifteen-minute worst-seed discovery gate. Use this measured rate,
      with explicit headroom, as the P2 throughput target.

Exit gate:

- We can distinguish a learning-quality failure from insufficient exploration,
  insufficient throughput, an action/observation gap, or an orchestration bug.
- The next campaign is justified by measured evidence rather than seed mining.

## P1 - Make learning improve search

- [ ] Verify that every decision exposes the complete state-local action
      surface. This includes analog directions and durations, roll, camera
      modifier, A/prompted actions, and any other primitive currently legal in
      native state. Unavailable actions must be explicitly unavailable, not
      silently absent.
- [ ] Audit the observation vector against retained native facts. It must expose
      supported velocity, trajectory history, momentum change, camera state,
      contacts and their measured kinematic consequences, prompted-action
      availability, action history, and terminal evidence with explicit
      missingness.
- [ ] Keep the objective lexicographic: first predict terminal reach/support,
      then conditional ticks-to-go. Budget-censored continuation is unknown,
      not a failed terminal. Do not encode `straight`, `roll`, wall avoidance,
      a waypoint, or any Ordon-specific desired behavior as reward.
- [ ] Demonstrate on held-out native state groups that pre-terminal learned
      ranking beats action-mean, scheduler-only, and random-valid controls.
      Report coverage, calibration, pairwise ordering, regret, uncertainty, and
      unsupported actions.
- [ ] Demonstrate in matched online campaigns that learned ranking reduces
      median unique useful expansions to the real terminal. Offline fit quality
      alone does not satisfy this task.
- [ ] Make exploration robust to the around-the-corner local optimum. Horizons
      must allow meaningful wandering; workers must lease diverse node/action
      expansions; uncertainty and coverage may prioritize search without
      becoming benchmark-specific utility.
- [ ] Prove that learner updates actually change future state-conditioned
      rankings at the intended cadence and that worker staleness stays within
      the sealed bound.
- [ ] Add the authenticated `131`-tick human replay as ordinary optional graph
      evidence and run an ablation. It may improve sample efficiency, but
      scratch success must remain possible and the replay must not become an
      incumbent policy, waypoint source, or privileged curriculum.

Exit gate:

- Learned ranking reaches the real terminal in fewer useful native expansions
  than matched non-learning controls on development and held-out seeds.
- The gain is attributable to generalization from surfaced state and action
  evidence, not a benchmark-specific heuristic.

## P2 - Make useful evidence arrive fast enough

- [ ] Complete one matched two-worker native measurement of batched replay
      publication. Require exact semantic work, proposal order, learner/replay
      authority, native ticks, and useful-expansion identity before accepting
      a performance result.
- [ ] Attribute and reduce the remaining fixed serial wall. The last measured
      run spent about `9.1` seconds in finalization and `8.3` seconds in replay
      publication. Eliminate repeated whole-history serialization,
      verification, compaction, or durable flushes; operational data remains
      binary and content-addressed.
- [ ] Profile tactic execution after persistence is reduced. Separate native
      simulation from checkpoint materialization, restore/capture, IPC,
      controller preparation, and worker idle time. Optimize the largest
      measured component rather than the aggregate timer.
- [ ] Make checkpoint reuse and dispatch scale across workers. Balance the
      selected proposal and counterfactual siblings, materialize each frontier
      no more often than required, preserve proposal order, and avoid evicting
      a useful live owner endpoint.
- [ ] Make model updates incremental or otherwise bounded if their cost grows
      with replay history. Preserve exact learner cadence and reproducible
      snapshots across restart.
- [ ] Run fixed-work 1/2/4/8/16-worker curves only after the current measured
      serial bottleneck is reduced. Report useful expansions/second, parallel
      efficiency, worker utilization, memory, cache eviction, replay lag,
      restore fallback, and phase occupancy.
- [ ] Retain a long-campaign resource audit proving bounded checkpoint memory,
      bounded learner staleness, and no history-dependent persistence or model
      growth that makes later decisions progressively slower.
- [ ] Exercise crash recovery before dispatch, during native execution, after
      native completion, after recovery commit, and after decision commit,
      including at least one fault after substantial graph/replay growth.
      Recovery must reproduce semantic work and account exactly for retries.
- [ ] Meet or exceed the throughput target derived in P0 on the reference
      workstation with enough headroom that learning and evaluation can share
      the wall budget.

Exit gate:

- The measured useful-expansion rate makes the P3 discovery time gate feasible.
- Increasing workers produces useful evidence rather than launch pressure,
  duplicate work, memory failure, stale learning, or coordinator serialization.

## P3 - Discover and improve the Ordon route

- [ ] Reach the actual load-zone predicate from scratch in all four sealed
      development seeds with median time to first terminal at or below five
      minutes and worst-seed time at or below fifteen minutes.
- [ ] Repeat scratch discovery on held-out seeds not used to repair the policy.
      Report terminal rate and uncertainty rather than treating four favorable
      seeds as general evidence.
- [ ] On the first terminal, immediately hand the exact successful graph path
      to optimization. Continue useful work instead of treating discovery as
      campaign completion.
- [ ] Expose branchable states across every successful path at intervals no
      coarser than four native ticks. Schedule counterfactuals from all supported
      interiors, not only option endpoints or the current best route.
- [ ] Rank post-terminal counterfactuals by predicted complete
      root-to-terminal ticks, uncertainty, and visits while retaining a fixed
      broad-exploration share. Exact native terminal returns remain truth.
- [ ] Demonstrate strict best-route improvement after first terminal in at
      least three sealed seeds.
- [ ] Beat the ordinary `131`-tick replay without making it incumbent or policy
      authority.
- [ ] Reach tick `123` or lower from scratch or optional-replay assistance.
- [ ] Cold-replay the selected tape at least twice with the learner out of the
      loop. Require identical controller bytes, first-hit tick, terminal
      evidence, source boundary, game/fixture identity, and execution fidelity.

Exit gate:

- Scratch discovery is reliable on development and held-out seeds.
- At least three seeds improve after their first terminal.
- A tick-`123`-or-lower route reproduces exactly in learner-free native replay.

## P4 - Discover and reuse tactics

- [ ] Mine parameterized action compositions from successful graph paths and
      high-value counterfactuals. Direction plus camera modifier, roll cadence,
      curved steering, and prompted actions must use the same generic
      composition mechanism.
- [ ] Preserve each composition's complete ordered primitive/parameter
      sequence, executable controller source, realized transition chain, and
      independent entry-state occurrences.
- [ ] Learn typed entry conditions from multiple independent source states
      without fabricating unsupported combinations of stage, procedure,
      contact, prompt, camera, or distance facts.
- [ ] Promote a composition only when native execution beats its primitive
      components on held-out state groups under the same horizon. Require
      paired benefit rather than aggregate wins that hide failures.
- [ ] Keep every valid primitive selectable after promotion and keep
      inapplicable promoted tactics out of the local action surface.
- [ ] Compare promotion-enabled and primitives-only search on held-out seeds
      using terminal rate, expansions/time to first terminal, expansions/time
      to best route, and useful expansions/second.
- [ ] Repeat discovery and improvement on a second native route problem without
      changing rewards or authoring route structure for that benchmark.

Exit gate:

- At least one learned composition provides reproducible held-out search value.
- The second route succeeds through the same state, action, learning, and
  orchestration contracts.

## P5 - Framework hardening and maintainability

- [ ] Provide a concise operator-facing campaign summary that identifies the
      dominant failure class and links to detailed content-addressed evidence.
      Operators should not need to inspect raw multi-megabyte reports to decide
      the next experiment.
- [ ] Audit hot-path ownership and data structures for accidental whole-graph,
      whole-replay, or whole-checkpoint cloning, hashing, serialization, and
      projection. Add growth tests for every repaired path.
- [ ] Split `native_tactic_route_runner/campaign.rs` by responsibility before
      adding more campaign behavior. It is currently 1,489 physical lines.
      Separate decision execution, durable commit/recovery, and
      finalization/report construction instead of treating the 1,500-line gate
      as a target.

Exit gate:

- Another engineer can reproduce a campaign, explain every selection and
  failure, resume it after interruption, and modify one subsystem without
  understanding an oversized monolith or breaking unrelated authority.

## Non-negotiable engineering constraints

- Every per-decision selection remains reconstructible from retained evidence:
  applicable actions, features, predicted support/ticks, uncertainty, visits,
  exploration ranks, learner revision, lease, native result, graph admission,
  and terminal/tick outcome.
- Coordinators may suspend, resume, cancel, and terminate only exact child
  handles they created. Process-name kills, global process scans, and
  ancestry-wide termination are not control mechanisms.
- The content-addressed state graph is the sole search authority. Replay,
  learner batches, indexes, reports, visualizations, and tactic mining are
  derived views; live endpoints and portable machine images are caches.
- Operational checkpoints, graph records, replay, journals, models, and learned
  tactics use versioned binary formats. JSON is limited to small authored
  requests and exported reports.
- Detached game/controller identity, fixture, source boundary, terminal
  predicate, action schema, feature schema, learner snapshot, graph state,
  replay, or execution fidelity fails closed.
- Production files stay below the enforced source-size limit and are split by
  responsibility before hot-path modules become dumping grounds.
- The clean-checkout audit validates formatting, source quality, tests,
  schemas, retained bundles, and replayability without launching unrelated
  native work.

## Experiment rules

- Every native run names the task and exit gate it tests.
- Run the smallest cell that can answer the question. After a failed
  architectural treatment, analyze it before launching another campaign.
- Controls share checkpoint, predicate, game/controller bytes, fixture, action
  and feature schemas, seeds, horizons, budgets, worker topology, and fidelity.
- Report sample efficiency and execution efficiency together: useful
  expansions to outcome and useful expansions per second.
- Do not mine seeds, hand-author routes, encode desired behavior as reward, or
  mistake faster failure for better learning.
- A terminal hit, a `125`-tick tie, or one isolated improvement is progress
  evidence, not acceptance.
- Retain detailed history in immutable benchmark bundles. Keep this file short,
  current, and executable.
