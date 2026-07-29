# Active tasks: build a coherent save-state learning planner

This file contains unfinished framework work only. Completed implementation
belongs in Git, and experimental history belongs in immutable benchmark
reports.

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
obstacle and hit the real load zone. If scratch learning cannot solve and
improve this in minutes rather than hours, the framework is not credible.

## Current diagnosis

The current implementation is not yet an accepted learning architecture.

- The durable content-addressed state graph now owns exact states, realized
  expansions, routes, and the best terminal path. Learner replay, executable
  frontiers, retained terminal artifacts, and completed-seed reports are
  validated projections of that graph.
- The live decision loop still selects work through legacy campaign/frontier
  policy instead of leasing registered node/action expansions from the graph
  scheduler. Until that is replaced, graph ownership does not yet produce
  coherent distributed graph search.
- Long options historically exposed only their endpoints. Four-tick native
  interior boundaries now exist, but they do not by themselves turn the
  system into coherent graph search.
- One isolated scratch route improved from tick `206` to tick `195`; controlled
  variants returned to tick `206`. This is not evidence of a reliable
  optimizer.
- The `131`-tick recording is a deliberately ordinary human sample, not the
  goal. Tick `123` or lower is the first credible Ordon result.

Do not spend more native hours varying seeds or acquisition ranks on the
current architecture. A native run is justified only when it tests a named
architectural invariant or an exit gate below.

## Non-negotiable design

### One authoritative state graph

The planner owns a durable, content-addressed directed graph:

- A node is an exact restorable native boundary, its complete typed state, and
  the evidence identity that binds them.
- An action expansion records one selected primitive or learned option and its
  complete native realization. Observed segments connect any interior
  boundaries without pretending that the continuing option was selected again.
- Interior boundaries of a long option are ordinary branchable nodes, but the
  observed continuation after an interior boundary is evidence, not a newly
  executable action.
- Merge nodes only when their future-affecting native state is proven
  equivalent. Tape frame, recent-action labels, or semantic proximity may be
  learner features but cannot by themselves prove a transposition. Keep all
  useful incoming evidence while using the fastest authenticated route among
  genuinely equivalent nodes for future restoration.
- Live process handles and portable machine images are caches for graph nodes,
  never alternative sources of search truth.
- Replay corpora, learner batches, semantic similarity indexes,
  visualizations, reports, and tactic mining are derived views of the graph.

There must not be a second behavior archive, replay list, or lane-local state
set with independent authority over which exact states exist or are
expandable.

### Learning ranks search; native evidence decides truth

- The optimization objective is lexicographic: reach the authenticated
  terminal, then minimize native ticks. Do not hide this behind a tunable
  terminal-reward constant.
- The learner predicts terminal reach/support and conditional ticks-to-go as
  distinct quantities. A budget-censored episode is unknown beyond its
  boundary, not evidence that the state can never succeed.
- The learned policy/value model ranks unexecuted node/action expansions. It
  never fabricates terminal support, replaces the policy-selected result after
  evaluation, or promotes a route.
- Once any terminal path exists, every node on that path receives an exact
  Monte Carlo ticks-to-go target. N-step/TD targets may generalize from it, but
  exact graph return remains separately inspectable.
- Before a terminal exists, exploration uses coverage, visit counts,
  reachability, model uncertainty, and prediction error as an explicit search
  policy. These are exploration priorities, not terminal reward or promotion
  authority.
- State inputs may expose velocity, trajectory history, momentum retention,
  camera state, contacts and their measured kinematic effects, prompted-action
  availability, and action history. Do not hand-code “straight,” “roll,” or
  any Ordon-specific behavior as utility.
- The learner must rank every currently executable primitive and promoted
  option. Unsupported estimates remain visibly unsupported.

### Discovery and optimization are distinct search regimes

Before the first terminal:

- independent workers lease diverse node/action expansions;
- the scheduler favors novel reachable states, uncertainty, and underexplored
  applicable actions;
- workers share graph evidence without sharing a single collapsed visitation
  frontier; and
- the discovery horizon is long enough for an unskilled trajectory to wander
  into the goal.

After the first terminal:

- the successful path is decomposed into exact branchable states at no coarser
  than four native ticks;
- the scheduler prioritizes counterfactual actions at those states by
  predicted total root-to-terminal ticks, uncertainty, and visit count;
- every shorter terminal path immediately updates exact returns and the best
  restoration route to its transposed states; and
- a fixed fraction of work continues broad exploration so the first slow path
  cannot become policy authority.

### Optional human replay is ordinary evidence

A human recording may add one authenticated path and its exact terminal return
to the same graph. Recorded controller segments are observations unless they
can be expressed by the ordinary live action space; loading a replay must not
silently add a recorded-tape action class. Human replay does not add authored
waypoints, privileged actions, behavior-cloning authority, a separate reward,
or a mandatory curriculum. Removing it must affect sample efficiency, not
whether the framework is capable of success.

### Operational data is binary and content-addressed

JSON is allowed for small authored requests and exported reports. It is not
the operational format for checkpoints, graph nodes, edges, replay, models,
journals, or learned tactics.

## P0 - Replace the accidental architecture with explicit ownership

- [ ] Create named modules with single ownership:
      `state_graph/`, `scheduler/`, `learner/`, `worker_pool/`, `tactics/`,
      `persistence/`, and `reporting/`.
- [ ] Keep production Rust files below 1,500 physical lines, with a normal
      target below 1,000. Split existing oversized route-runner and campaign
      files by responsibility before adding more policy variants.
- [ ] Add invariant tests proving that graph, learner corpus, restored state,
      terminal path, and exported report all name the same content identities.

Exit gate:

- A code audit can trace one native expansion from scheduler lease through
  worker execution, graph admission, learner publication, and report without
  consulting a second source of search truth.
- Restarting from the durable graph produces the same pending expansions and
  best terminal path.

## P1 - Implement exact save-state graph search

- [ ] Store enough route/checkpoint evidence to restore any expandable node;
      validate the restored typed state before executing from it.
- [ ] Implement conservative transposition detection from validated
      future-equivalence evidence and fastest-route relaxation. Semantic state
      similarity may share learning but must not merge restorable nodes. Route
      improvement to an equivalent node must propagate to descendant total
      costs without deleting alternate incoming evidence.
- [ ] Use bounded leases or virtual loss so workers do not duplicate the same
      expansion while independent exploration remains diverse.
- [ ] Implement a deterministic priority queue whose decision can be replayed
      from graph state, learner snapshot, seed, and sealed scheduler config.
- [ ] Add a small deterministic around-the-corner environment with a known
      shortest path. Prove exhaustive mode finds it and transpositions reduce
      duplicate work.

Exit gate:

- A terminal trajectory containing a 40-tick option exposes all configured
  interior nodes, and a counterfactual action can execute from each one.
- The deterministic fixture reaches its known shortest path after a bounded
  number of unique expansions.

## P2 - Make learning a serious expansion policy

- [ ] Write and test the algorithm contract: state/action inputs, terminal
      support target, censored target, conditional tick-return target,
      bootstrap rule, uncertainty estimate, target-network/update cadence, and
      deterministic ranking tuple.
- [ ] Define one action-conditioned learner interface over typed node features,
      applicable action factors, graph visits, and exact/n-step returns.
- [ ] Fit terminal ticks-to-go from complete successful graph paths and keep
      exact targets separate from generalized predictions.
- [ ] Treat horizon exhaustion and cancellation as censored evidence. Only an
      authenticated terminal or explicit terminal failure may close a return.
- [ ] Train auxiliary predictions for next-state features, realized duration,
      action acceptance, prompted-action availability, and terminal
      probability. Use their errors/uncertainty for representation and
      exploration, not hand-authored route reward.
- [ ] Use held-out state groups and independently realized actions to measure
      value calibration and ranking quality. In-sample fit is not evidence.
- [ ] Implement prioritized replay over surprising, rare, terminal-connected,
      and policy-relevant graph edges without starving ordinary evidence.
- [ ] Compare at least one stable discrete/action-factor baseline against the
      current k-NN and Double-Q treatments. Delete treatments that lose the
      sealed calibration and native search controls.
- [ ] Prove on the deterministic fixture that the learned scheduler reaches
      the shortest path in fewer expansions than exhaustive and non-learning
      search without losing completeness.

Exit gate:

- The learner measurably reduces unique expansions and wall time on held-out
  fixture seeds.
- Exact terminal return, generalized value, uncertainty, and exploration
  priority are separately inspectable for every scheduled expansion.

## P3 - Make throughput an architectural property

- [ ] Measure process launch, simulation, state extraction, direct restore,
      replay restore, graph admission, learner update, IPC, persistence, and
      reporting separately.
- [ ] Establish fixed-work curves at 1, 2, 4, 8, and 16 workers using unique
      useful graph expansions, not raw ticks, as throughput.
- [ ] Keep workers and game processes persistent across expansions.
- [ ] Use compact binary batches and shared content identities; do not send
      repeated snapshots or JSON transition blobs through IPC.
- [ ] Verify which rendering, audio, presentation, and proof systems may be
      disabled without changing native state or terminal evidence. Retain
      measured parity evidence for every disabled subsystem.
- [ ] Profile restore locality and schedule leases to checkpoint owners when it
      reduces replay without collapsing exploration diversity.
- [ ] Deliver a 10x reduction in wall time to a fixed useful-evidence target
      against the recorded baseline, or identify and remove the measured
      saturation bottleneck before proposing more capacity.

Exit gate:

- Additional workers increase unique useful expansions per second with bounded
  memory and learner staleness.
- The native integration tests below can complete within their stated
  wall-time gates on the reference workstation.

## P4 - Achieve practical scratch discovery

- [ ] Run Ordon from the authenticated source with no human replay, incumbent
      tape, authored coordinate, or promoted Ordon tactic.
- [ ] Give discovery at least 30 seconds of possible native route horizon, but
      cap total graph expansions and wall time.
- [ ] Reach the actual load-zone predicate in all four sealed scratch seeds.
- [ ] Achieve median time-to-first-terminal at or below five wall minutes and
      worst-seed time at or below fifteen wall minutes on the reference
      workstation.
- [ ] Report unique graph nodes/edges, duplicate transpositions, leases,
      restores, simulated ticks, terminal paths, learner work, and wall time.
- [ ] Demonstrate that adding an ordinary human replay improves median
      time-to-first-terminal while remaining unnecessary for scratch success.

Exit gate:

- Walking off the map is a reliable minutes-scale operation, not an overnight
  search.
- Failure to hit the time gate sends work back to P1/P2/P3; it does not
  authorize more seed mining.

## P5 - Reliably improve successful routes

- [ ] From each newly successful path, schedule counterfactual expansions
      across the complete interior-state sequence rather than option endpoints
      alone.
- [ ] Demonstrate repeated monotonic best-route improvement in at least three
      sealed seeds. One favorable counterfactual is not proof.
- [ ] Show that learned total-tick ranking selects better counterfactuals than
      visit-count, random-valid, and exhaustive local controls at the same
      expansion budget.
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
      using terminal rate, time-to-first-terminal, time-to-best-route, and
      unique useful expansions per second.
- [ ] Repeat the complete discovery/improvement evaluation on a second native
      route problem before claiming a generic framework.

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
- Reports distinguish terminal discovery, promotion, best authenticated tick,
  exact graph return, generalized prediction, and exploration priority.
- A faster run of an unchanged failing algorithm is throughput evidence, not
  learning success.
- A terminal hit, a tick-`125` tie, or one tick-`195` scratch route is progress
  evidence only.
