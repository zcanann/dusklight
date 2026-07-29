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
- The graph admits content-canonical future-equivalence proofs only from its
  configured native validator. It retains every route-specific exact node and
  incoming segment, chooses the fastest authenticated member for restoration,
  and propagates relaxed costs through descendants for scheduling; binary
  restart reproduces the proof classes and relaxed costs.
- Every executable node can now issue a compact content-bound restoration plan
  naming its exact typed state, portable route, and optional native boundary.
  The graph rejects a worker completion unless the restored complete typed
  state validates against that plan; altered state or route evidence fails
  closed.
- Every live graph expansion now carries that plan and its typed-state receipt
  into native dispatch. Uncached non-root nodes run a replay-only
  materialization request and compare the complete reconstructed
  `FactSnapshot` before the selected action executes; cached live endpoints
  remain usable only when their state, route checkpoint, tape digest, and
  native boundary fingerprint all match.
- The live decision loop now registers horizon-eligible actions and completes
  exact graph leases. Outside the explicit human-demonstration curriculum,
  the graph scheduler also chooses which exact node to restore by coverage in
  discovery and exact successful-path membership in optimization. Action
  ranking consumes exact heads and a censored-safe, state-conditioned
  conditional-tick estimate when terminal-connected examples exist; otherwise
  it falls back to the existing policy order. Generalized terminal probability
  remains unsupported rather than being fabricated from censored continuations.
- Expansion scheduling now seals graph identity, learner-priority snapshot,
  regime, seed, generation, lease policy, complete queue order, and selected
  expansion into one replayable decision. Binary graph restart reproduces the
  same queue exactly, and leasing consumes the graph-owned expansion rather
  than mutating scheduler-local work.
- The coherent planner now has explicit `state_graph`, `scheduler`, `learner`,
  `worker_pool`, `tactics`, `persistence`, and `reporting` ownership modules.
  One invariant audit traces a compact leased job through typed restored-state
  validation, graph completion, exact terminal selection, learner publication,
  durable binary restart, and JSON report using the same graph and expansion
  content identities.
- A repository-wide Rust source audit now excludes only explicit test modules
  and finds no production file at or above 1,500 physical lines; the largest is
  `tactic_q_campaign.rs` at 1,499. Route planning, search, playback, corpus
  inspection, semantic evaluation, transition validation, and CLI dispatch are
  split along explicit ownership boundaries rather than extended as monoliths.
- Exact terminal returns now come from route-specific graph identities, and
  the learner contract distinguishes authenticated terminal reconnection from
  an open censored continuation. An exact state/action table consumes those
  targets as a stable control and publishes separate scheduler heads.
  Generalized, continuous, and categorical value fits now train objective
  return only on authenticated terminal-connected rows, using negative native
  ticks-to-terminal. Horizon/cancellation tails remain unsupported rather than
  becoming scalar-reward failures.
- Graph learner rows now carry exact source/next feature vectors, realized
  duration, option end reason, observed action acceptance, prompted-action
  status, and immediate terminal outcome. The exact table retains exact
  predictions and fits an action-shared delta/rate baseline whose numeric
  prediction error is published to the scheduler as exploration evidence.
- A content-bound calibration report now withholds whole exact-state groups,
  distinguishes independently realized from unseen actions, measures
  auxiliary and conditional-tick error, and compares pairwise action ranking
  against an action-mean baseline. Its gate requires complete independently
  realized objective coverage, bounded held-out error, and positive error and
  ranking gains; unsupported data remains explicit zero coverage.
- A content-bound graph replay plan assigns inspectable surprise, rarity,
  terminal-connection, and active-policy signals to authoritative realized
  edges. A sealed ordinary-evidence lane rotates across all rows with a bounded
  starvation interval; the remaining deterministic weighted draws feed the
  generalized auxiliary and conditional-tick fits.
- A graph-native treatment comparison withholds the same exact states from a
  stable discrete action-mean control, state-kNN, and discrete Double-Q. It
  publishes coverage, tick error, and pairwise ranking for every treatment and
  selects only a treatment that passes the sealed thresholds; the scheduler
  retains state-kNN while the losing controls remain non-authoritative.
- A deterministic around-the-corner fixture has a known unique nine-tick
  optimum and content-addressed exact states. Exhaustive search proves the
  optimum, exact transpositions reduce duplicate expansions, and a
  return learner trained only from realized transitions ranks the optimum in
  `40` rather than `101` unique expansions across four graph-disjoint
  translated fixture seeds without pruning unsupported work. An
  order-balanced 128-repetition comparison also requires lower measured wall
  time from the learned treatment.
- Every returned scheduled expansion now exposes the exact authenticated
  terminal return of its source, its separately generalized
  action-conditioned tick estimate, uncertainty, and its final deterministic
  queue rank. Unsupported exact or generalized heads remain explicit `None`
  rather than being inferred from queue position.
- Native expansion reports now authenticate the child process's restore,
  simulation, observation-capture, and corpus-encoding phase profile, while
  host accounting separately measures process launch, IPC/result transport,
  Rust state extraction, direct and replay restore, graph admission, learner
  update, persistence, and report serialization.
- Long options historically exposed only their endpoints. A graph invariant
  now admits a 40-tick realization as ten observed segments with nine ordinary
  four-tick interior nodes, then independently executes a counterfactual
  expansion from every interior node without relabeling observed continuation
  as a selected action.
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

- [x] Keep production Rust files below 1,500 physical lines, with a normal
      target below 1,000. Split existing oversized route-runner and campaign
      files by responsibility before adding more policy variants.

Exit gate:

- A code audit can trace one native expansion from scheduler lease through
  worker execution, graph admission, learner publication, and report without
  consulting a second source of search truth.
- Restarting from the durable graph produces the same pending expansions and
  best terminal path.

## P1 - Implement exact save-state graph search

- [x] Store enough route/checkpoint evidence to restore any expandable node;
      validate the restored typed state before executing from it.
Exit gate:

- A terminal trajectory containing a 40-tick option exposes all configured
  interior nodes, and a counterfactual action can execute from each one.
- The deterministic fixture reaches its known shortest path after a bounded
  number of unique expansions.

## P2 - Make learning a serious expansion policy

- [x] Use held-out-generalizing action-conditioned return and uncertainty to
      rank graph-owned expansion work, with complete per-expansion decision
      evidence.

Exit gate:

- The learner measurably reduces unique expansions and wall time on held-out
  fixture seeds.
- Exact terminal return, generalized value, uncertainty, and exploration
  priority are separately inspectable for every scheduled expansion.

## P3 - Make throughput an architectural property

- [x] Measure process launch, simulation, state extraction, direct restore,
      replay restore, graph admission, learner update, IPC, persistence, and
      reporting separately.
- [x] Establish fixed-work curves at 1, 2, 4, 8, and 16 workers using unique
      useful graph expansions, not raw ticks, as throughput.
- [x] Keep workers and game processes persistent across expansions.
- [ ] Use compact binary batches and shared content identities; do not send
      repeated snapshots or JSON transition blobs through IPC.
- [ ] Verify which rendering, audio, presentation, and proof systems may be
      disabled without changing native state or terminal evidence. Retain
      measured parity evidence for every disabled subsystem.
- [ ] Profile restore locality and schedule leases to checkpoint owners when it
      reduces replay without collapsing exploration diversity.
- [x] Deliver a 10x reduction in wall time to a fixed useful-evidence target
      against the recorded baseline, or identify and remove the measured
      saturation bottleneck before proposing more capacity.

The 2026-07-29 macOS fixed-work curve held every cell to one decision and 16
unique useful graph expansions. Median wall time fell from 103.322 seconds at
one worker to 65.722 seconds at eight workers, then regressed to 85.211 seconds
at sixteen. All ten runs had zero replay staleness and the same semantic
expansion/evidence digest. Process launch and native boot consumed 54.4-80.7
seconds per cell while useful execution fell from 45.1 seconds to 3.8 seconds;
the next throughput treatment must therefore remove launch/boot saturation,
not add workers. The sealed report content SHA-256 is
`916bb1bd02827c590ccef1cfc849d30d19968c3928a75c03f4d76ca61fb2a579`;
the post-run expansion-set audit SHA-256 is
`8bc738c30fbd903dc0c62799c304d809edb9797a99a529bb861ba336e6f4e432`.

The v2 treatment keeps one authenticated 16-process fleet alive across every
cell and varies only the active worker prefix. It paid 79.465 seconds of launch
once, then every sample reported zero launch time. Median steady-state wall
time now falls monotonically from 45.478 seconds at one worker to 5.485
seconds at sixteen, or 2.917 useful expansions per second. Against the prior
same-topology 85.211-second median, the fixed 16-expansion target is 15.54x
faster. Including the one fleet boot, the complete ten-cell treatment took
270.212 seconds instead of 811.235 seconds; the one-time startup remains
reported rather than hidden. All cells retained the same expansion/evidence
digest with zero staleness. The sealed v2 report content SHA-256 is
`1499e563b4fb308b005e88c0fb7ff60df283edce4bfe9c47c37d0755cf656aaa`.

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
