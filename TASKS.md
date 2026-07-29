# Active tasks: build a credible save-state learning planner

This file contains unfinished framework work only. Completed implementation
belongs in Git, and experimental history belongs in immutable benchmark
artifacts.

## Product objective

Build a generic system that can learn efficient controller routes through a
deterministic game from an authenticated native checkpoint.

The environment gives the system:

- exact restorable game states;
- typed observations and currently applicable actions;
- controller inputs and their realized trajectories;
- a terminal predicate; and
- native ticks consumed.

The system must discover a terminal route, improve it, and learn reusable
action compositions without authored route coordinates, benchmark-specific
rewards, or UI-authored TAS graphs.

The Ordon acceptance problem is intentionally elementary: walk around one
obstacle and hit the real load zone. The ordinary human recording reaches it
at tick `131`; tick `123` or lower is the first credible optimization result.
If scratch learning cannot solve and improve this in minutes rather than
hours, the framework is not credible.

## Evidence snapshot and present diagnosis

The incoming 2026-07-29 work made substantial, useful progress:

- The durable content-addressed state graph owns exact states, realized
  expansions, restoration plans, terminal paths, learner rows, scheduling
  decisions, persistence, and reports.
- Exact restored typed state is validated before graph-owned native expansion.
  Long options expose branchable interior states, and deterministic tests cover
  transpositions, restart, lease lifecycle, and exact terminal returns.
- A deterministic around-the-corner fixture has a known nine-tick optimum. A
  learner trained on graph-disjoint translated fixtures reaches it in `40`
  unique expansions versus `101` for the non-learning control. This is good
  plumbing evidence, not native-policy acceptance.
- Persistent native workers, compact repeated-request transport, subsystem
  suppression parity, phase timing, and checkpoint-owner locality are
  implemented. A short macOS fixed-work treatment reports a 15.54x steady-state
  speedup over relaunching the same topology.
- The Rust source-quality gate has zero debt exemptions and all 532 production
  files are below 1,500 physical lines. The four final oversized orchestration
  files are split by result tests, launch preparation, proposal-pool execution,
  frontier policy, and checkpoint validation responsibilities. The complete
  orchestration suite is hermetic and passes all 256 tests.
- Scratch validation now publishes a movable content-addressed bundle carrying
  request, execution, plan, route, per-seed, graph checkpoint, terminal
  tape/result, and source-authority evidence. The clean-checkout audit command
  validates source quality, formatting, the workspace, all orchestration tests,
  and every committed bundle.
- A bounded macOS scratch campaign reached the actual Ordon load-zone
  predicate in all four configured seeds without human replay. Its best route
  was `231` ticks, median time to first terminal was about `691.6` seconds, and
  worst time was about `728.3` seconds.

That campaign proves bounded terminal discovery under its exact conditions. It
does not prove practical discovery, useful native learning, route
optimization, cross-platform parity, or a generic framework.

The current evidence cannot yet tell whether the limiting factor is:

- poor expansion ranking or insufficient exploration;
- too few useful expansions per second;
- restore, IPC, learner, persistence, or scheduler contention during a real
  campaign;
- action availability or observation features that make good behavior hard to
  infer;
- a discovery-to-optimization handoff that stops exploiting useful terminal
  evidence; or
- correctness bugs hidden by incomplete retained evidence and failing quality
  gates.

Do not answer this by seed mining. Run the matched diagnostics below before
another long campaign.

## Non-negotiable design

### One authoritative state graph

- A node is an exact restorable native boundary, its complete typed state, and
  the evidence identity that binds them.
- An action expansion records one selected primitive or learned option and its
  complete native realization. Observed segments connect interior boundaries
  without pretending the continuing option was selected again.
- Merge nodes only when future-affecting native state is proven equivalent.
  Semantic proximity is a learner feature, not transposition authority.
- Live process handles and portable machine images are caches for graph nodes,
  never alternative sources of search truth.
- Replay, learner batches, indexes, reports, visualizations, and tactic mining
  are derived graph views. There must not be a second authoritative frontier
  or behavior archive.

### Learning ranks search; native evidence decides truth

- The objective is lexicographic: reach the authenticated terminal, then
  minimize native ticks. Do not encode it as a tunable terminal-reward
  constant.
- Predict terminal reach/support and conditional ticks-to-go separately.
  Budget-censored continuation is unknown, not a failed terminal proof.
- Learned values rank unexecuted node/action expansions. They never fabricate
  terminal support, replace the executed policy result, or promote a route.
- Exact successful graph paths provide exact ticks-to-go targets. Generalized
  estimates and uncertainty remain separately inspectable.
- Before a terminal exists, exploration may use coverage, visits, reachability,
  uncertainty, and prediction error. These are search priorities, not
  benchmark-specific reward.
- State may expose velocity, trajectory history, momentum retention, camera
  state, contacts and measured kinematic effects, prompted-action availability,
  and action history. Do not hand-code “straight,” “roll,” wall avoidance, an
  Ordon waypoint, or any other desired behavior as utility.
- Every currently applicable primitive and promoted option remains selectable.
  Unsupported estimates remain visibly unsupported.

### Discovery and optimization are distinct regimes

Before the first terminal, workers lease diverse node/action expansions and
share graph evidence without collapsing their local coverage frontiers.
Exploration horizons must be long enough for an unskilled route to reach the
goal.

After the first terminal, immediately decompose the successful path into exact
branchable states at no coarser than four native ticks. Rank counterfactuals by
predicted total root-to-terminal ticks, uncertainty, and visits while retaining
a fixed broad-exploration share. A slow first route is evidence, not policy
authority.

### Optional human replay is ordinary evidence

An authenticated human replay may add one path and exact terminal return to the
same graph. It adds no authored waypoint, privileged action, behavior-cloning
authority, separate reward, or mandatory curriculum. Removing it may reduce
sample efficiency; it must not remove the framework's ability to succeed.

### Operational data is binary and content-addressed

JSON is allowed for small authored requests and exported reports. It is not the
operational format for checkpoints, graph nodes, edges, replay, models,
journals, or learned tactics.

## P0 - Restore a truthful, auditable baseline

- [ ] Recover the original `231`-tick macOS campaign artifacts or rerun its
      sealed execution plan, then commit its self-contained scratch evidence
      bundle. The existing summary contains only an unavailable child-report
      hash and cannot be upgraded without the originating per-seed graph,
      checkpoint, tape, and terminal-result artifacts.

Exit gate:

- A clean checkout passes the documented audit command.
- Another engineer can independently validate the `231`-tick claim and every
  per-seed terminal without access to the originating macOS build directory.

## P1 - Diagnose the `231`-tick campaign before changing the algorithm

- [ ] Produce a per-seed campaign audit with time and unique useful expansions
      to first terminal and best terminal, selected-node and selected-action
      reasons, exploration versus learned selections, restores and fallbacks,
      learner revisions consumed, terminal-path lengths, and stop reasons.
- [ ] Compare each discovered route with the `131`-tick ordinary replay at the
      level of trajectory, velocity retention, action availability, roll/camera
      use, option duration, detour, contact-correlated slowdown, and idle or
      repeated work. This is feature/action-surface diagnosis, not permission
      to encode the human route as reward.
- [ ] Run matched scratch cells for learned ranking, scheduler-only coverage,
      and random-valid ranking. Hold source, terminal, action schema, seeds,
      horizons, proposal width, workers, and expansion/wall budgets fixed.
- [ ] For those cells report both sample efficiency and execution efficiency:
      terminal rate, useful expansions to terminal, useful expansions per
      second, simulated ticks per expansion, restore time, native simulation,
      IPC, graph admission, learner update, persistence, and idle/saturation.
- [ ] Reproduce one sealed diagnostic cell on macOS and the reference
      workstation with identical game/controller bytes and execution fidelity.
      Explain any difference in terminal, route tick, action applicability, or
      throughput before comparing learning treatments across platforms.

Exit gate:

- Evidence identifies whether the primary failure is expansion quality,
  expansion throughput, restoration/orchestration, action/feature coverage, or
  platform drift. More native work is then assigned to that measured cause.

## P2 - Prove native learning improves search

- [ ] Demonstrate on held-out native state groups that the pre-terminal learner
      ranks independently realized actions better than action-mean,
      scheduler-only, and random-valid controls. Report coverage, calibration,
      pairwise ordering, uncertainty, and unsupported actions.
- [ ] Demonstrate that learned ranking reduces median unique useful expansions
      to the real terminal under matched budgets. The deterministic
      around-the-corner fixture remains a regression test, not this gate.
- [ ] After terminal support appears, demonstrate that learned total-tick
      ranking selects better counterfactuals than visit-count, random-valid,
      and exhaustive-local controls on held-out successful-path states.
- [ ] Audit the observation vector against native evidence. Applicable actions,
      velocity and trajectory history, camera state, prompted actions, and
      kinematic consequences such as lost speed must be available when
      supported by state; no derived signal may directly encode desired Ordon
      behavior.
- [ ] Seal every scheduler decision with exact return, generalized terminal
      support, conditional ticks, uncertainty, exploration priority, model
      revision, queue, and final selection so a binary graph restart reproduces
      it exactly.

Exit gate:

- Learning reduces native expansions to terminal and improves held-out
  counterfactual selection. A faster unchanged coverage heuristic does not
  satisfy this gate.

## P3 - Make real-campaign throughput scale

- [ ] Run fixed-work curves at 1, 2, 4, 8, and 16 workers over enough decisions
      to exercise graph growth, repeated restores, learner updates,
      persistence, and bounded checkpoint eviction. A one-decision warm-fleet
      microbenchmark is necessary but not sufficient.
- [ ] Account for every scheduled lease as completed, retryable, cancelled, or
      failed. Report proposal dispatches separately from graph expansions and
      observed interior segments so throughput denominators cannot be inflated.
- [ ] Bound memory, learner staleness, replay fallbacks, process loss, and
      checkpoint-owner skew during long campaigns. Prove crash recovery cannot
      silently lose or duplicate graph work.
- [ ] Remove the measured end-to-end saturation bottleneck until the reference
      workstation sustains enough useful evidence to meet the P4 wall-time
      gate. Prefer reducing restore/replay and idle time before adding capacity.
- [ ] Preserve native state, applicable actions, controller output, terminal
      evidence, and first-hit tick for every disabled presentation subsystem.

Exit gate:

- Additional workers improve long-running useful expansions per second with
  bounded memory and staleness.
- The P4 time gate is feasible at the measured expansions-to-terminal count,
  rather than extrapolated from a short microbenchmark.

## P4 - Make scratch discovery practical and general

- [ ] Reach the actual load-zone predicate in all four sealed evaluation seeds
      with median time to first terminal at or below five wall minutes and
      worst-seed time at or below fifteen minutes on the reference workstation.
- [ ] Repeat the acceptance run on held-out seeds that were not used to repair
      or tune the discovery policy. Report terminal rate and confidence rather
      than treating the four development seeds as generic evidence.
- [ ] Demonstrate that adding the ordinary `131`-tick human replay improves
      median time or expansions to first terminal while remaining unnecessary
      for scratch success.
- [ ] Retain the complete P0 evidence bundle for every acceptance run.

Exit gate:

- Walking off the map is a reliable minutes-scale operation, not an overnight
  search.
- Failure sends work back to the measured P1/P2/P3 cause; it does not authorize
  more seed mining.

## P5 - Reliably improve successful routes

- [ ] Continue useful work after first terminal instead of treating discovery
      as campaign completion. Hand the exact successful graph path and terminal
      returns directly to the optimization scheduler.
- [ ] Schedule counterfactuals across the complete interior-state sequence of
      every newly successful path, not option endpoints alone.
- [ ] Demonstrate repeated monotonic best-route improvement from the first
      terminal in at least three sealed seeds. One favorable counterfactual is
      not proof.
- [ ] Beat the `131`-tick ordinary demonstration without making it incumbent or
      policy authority.
- [ ] Reach tick `123` or lower from scratch or optional-replay assistance,
      then cold-replay the complete tape twice with the learner out of loop.
- [ ] Require identical controller bytes, first-hit tick, terminal evidence,
      game identity, fixture, source boundary, and execution fidelity.

Exit gate:

- At least three sealed seeds improve their first discovered terminal route.
- The best authenticated route is tick `123` or lower and reproduces exactly.

## P6 - Discover and reuse tactics

- [ ] Mine parameterized action compositions from successful graph paths and
      high-value counterfactuals. Direction plus camera modifier, roll cadence,
      curved steering, and prompted actions use the same generic mechanism.
- [ ] Learn typed entry conditions from independent source states.
- [ ] Promote only when a composition improves terminal/tick return on held-out
      state groups relative to executing its primitive components.
- [ ] Keep every valid primitive selectable after promotion.
- [ ] Compare promotion-enabled and primitives-only search on held-out seeds
      using terminal rate, time to first terminal, time to best route, and
      unique useful expansions per second.
- [ ] Repeat the complete discovery and improvement evaluation on a second
      native route problem before claiming a generic framework.

Exit gate:

- At least one learned composition provides reproducible held-out search value.
- The second route problem succeeds without benchmark-specific reward or
  authored route structure.

## Experiment discipline

- Every native run names the architectural invariant or exit gate it tests.
- Do not run more than one diagnostic cell after a failed architectural
  treatment. Analyze or redesign before spending another full campaign.
- Controls share source checkpoint, terminal predicate, game bytes, fixture,
  route horizon, expansion budget, worker topology, and fidelity.
- Reports distinguish terminal discovery, route promotion, best authenticated
  tick, exact graph return, generalized prediction, and exploration priority.
- Report wall time and unique useful expansions together. Faster failure is
  throughput evidence; fewer expansions at unchanged throughput is policy
  evidence.
- A terminal hit, a `125`-tick tie, or one isolated route improvement is
  progress evidence only.
