# Generic in-game goal learner

## Objective

Build a generic learner that discovers complete, substantially better-than-human
solutions to native game goals at useful speed. The algorithm is not prescribed:
value learning, graph search, planning, imitation, and learned options may be
combined when experience—not authored routes—causes better decisions.

The learner receives only:

- a binary root save state and restorable states produced by its own experience;
- a native terminal predicate and native tick cost;
- typed observations such as motion, collision response, camera, input history,
  game mode, procedure, and currently available prompted actions;
- primitive, simultaneous, variable-duration, and learned/promoted input options.

It must not receive route coordinates, route-specific rewards, named solutions,
or a privileged action sequence. Generic observations are evidence to learn
from, not individually rewarded rules. Human demonstrations are optional
experience: the learner must work without them and must be able to surpass them.

Ordon Springs is the first native adequacy test. Success is the real
`ordon_spring_load_committed` terminal. The human sample reaches it in 125 active
ticks; the first pass gate is 124, convincing evidence is 123, and the current
target is 120.

## Current evidence

- The production online loop learns shortest paths in two exact-state-disjoint
  deterministic around-corner worlds, branches from retained states, and backs
  up exact terminal cost. This proves basic plumbing, not native adequacy.
- The native adapter retains bounded process-local checkpoints and has directly
  restored multiple nonconsecutive intermediate states. Save-state branching is
  functional; useful optimization from those branches is not yet proven.
- The best authenticated native route is 210 ticks and cold-replays twice at the
  same real terminal tick. It is a real improvement over the former 318-tick
  diagnostic route, but it is nowhere near the 124-tick adequacy gate.
- The former native optimizer's forced perturbation, coordinate rejoin, and
  privileged incumbent continuation have been removed. Ordinary checkpoint
  refinement now executes only learner-selected actions until terminal or its
  shared tick budget.
- Native diagnostics exposed and fixed three policy-path defects: scheduler
  leases substituted merely applicable actions for the policy choice;
  width-one proposal batches could not select learned estimates; and immature,
  below-chance estimates controlled too many actions. Adaptive estimates now
  replace only unsupported bootstrap, retain epsilon and exact-greedy
  authority, and require sufficient above-chance evidence before regular use.
- A matched 64-decision diagnostic produced only a 406-tick route because
  episode-based acquisition gave rank-zero optimization 27 of 790
  post-terminal action ticks (3.4%). Scheduling now accounts actual evaluated
  native work and gives new single-lane plans an equal support/discovery
  envelope. A bounded two-proposal revalidation stopped at 48 decisions and
  found a 274-tick terminal from an initial-episode sibling. Afterward it spent
  347 ticks on support and 342 on discovery, proving the allocation correction,
  but no restored branch improved 274. The trace exposed terminal-basin churn:
  fresh suffixes from slower 286--320 tick successes displaced already-expanded
  boundaries on the best 275-tick graph lineage. Graph scheduling now compares
  authenticated total route cost before coverage, matching the learned-frontier
  scheduler. A cached rank-zero source may receive one immediate reuse only
  when it belongs to that best authenticated total, and it retains the exact
  incumbent ticks-to-go bound. Locality consumes the most recently materialized
  eligible source before older pending cache entries. All 489 orchestration and
  453 learning tests pass. A native restored-branch improvement remains the
  required revalidation.
- Tactic mining, validation, promotion, binary persistence, and ordinary policy
  selection exist on the production path. No native campaign has yet shown a
  learner-discovered promoted tactic improving a route.

No task is blocked on design.

## P0 — make native checkpoint learning coherent

- [ ] Make retained checkpoints the actual unit of optimization. Demonstrate in
  the production loop that at least two nonconsecutive intermediate states are
  restored without root replay and that ordinary subsequent decisions from one
  produce a faster authenticated terminal route.
- [ ] Verify that terminal returns change later action ranking at predecessor
  states and propagate far enough to solve temporarily-worse moves around a
  corner. Open rollouts remain censored but still teach availability, dynamics,
  duration, novelty, and uncertainty.
- [ ] Keep exploitation and discovery live without decision-count scripts or
  forced route phases. Learned terminal value should revisit promising prefixes;
  uncertainty and state/action coverage should allocate unsupported trials.
- [ ] Expose simultaneous and variable-duration primitives—including movement,
  camera lock, roll, and their legal compositions—through the same state-aware
  action interface. Availability must come from native state, not a route script.
- [ ] Mine repeated useful subsequences, validate them against their primitive
  realization from matched retained states, and promote only improvements that
  generalize across compatible states. Primitives remain available.

Exit: the production learner, without a privileged route continuation, learns
the shortest solutions in deterministic checkpoint worlds and improves a native
incumbent from a nonconsecutive restored checkpoint.

## P0 — beat the Ordon route

- [ ] After the coherence exit above, run one bounded seed-104729 diagnostic that
  answers multiple hypotheses in one scorecard: terminal discoveries, incumbent
  improvements, branch sources, completed and censored rollouts, restore modes,
  action/tactic coverage, learned ranking changes, useful experience per second,
  and cold-replay result. Diagnose the first failed learning decision before
  adding campaign volume.
- [ ] Discover and cold-replay a zero-shot route of 124 ticks or less twice with
  identical terminal identity and first-hit tick.
- [ ] Reach 123 ticks or less and then 120 ticks or less with unchanged generic
  observations, objective, action interface, and learning rules.
- [ ] Under one bounded envelope, run five fixed zero-shot seeds and a permuted
  seed order. All must reach the terminal; route quality and discovery time must
  not depend on one lucky seed, scheduler ordering, or cross-run leakage.

Exit: the framework reliably discovers substantially better-than-human Ordon
routes. Merely finding the terminal or approaching 125 ticks does not pass.

## P0 — prove learning and learned tactics caused the result

- [ ] Compare an improving adaptive run with frozen-policy and seeded
  random-valid controls over matched checkpoints, opportunities, seeds, and
  native-tick budgets. Require a held-out gain in terminal rate, discovery time,
  route ticks, or useful expansions.
- [ ] At decisive checkpoints, show which experience changed action ranking and
  how generic trajectory, momentum, collision response, camera, input history,
  and prompted-action observations affected the learned estimate. Do not turn
  those observations into authored reward bonuses.
- [ ] Show a learner-discovered, promoted multi-action tactic improving held-out
  native decisions over its primitive components.
- [ ] Ablate an ordinary suboptimal human demonstration. It may accelerate
  learning, but must not be required, cap the policy, or supply tactics absent
  from the demonstration.

Exit: adaptive learning and reusable learned tactics—not lucky search, a forced
splice, or an authored route—causally improve native results.

## P0 — make successful learning fast enough

- [ ] Profile the coherent end-to-end learner once on a representative growing
  corpus. Attribute time and memory to native execution, restore, model update,
  graph scheduling, evidence admission, persistence, and IPC.
- [ ] Optimize only measured dominant costs while preserving exploration and
  evidence semantics. Re-run the same treatment and require materially more
  useful experience per minute or less time to the same learned result.
- [ ] Compare one and two checkpoint-owning lanes. Retain parallelism only when
  it increases unique useful experience without contamination, host saturation,
  or interference with unrelated processes.

Exit: the framework explains where time goes and reaches the same learning
quality materially faster. Raw attempts per second alone do not pass.

## P1 — generality and engineering quality

- [ ] Apply the unchanged goal, observation, action, learning, and tactic
  interfaces to a second native goal without route-specific shaping or tactics.
- [ ] Decompose mixed-responsibility production files into independently testable
  execution, checkpoint, branching, learning, proposal, tactic, persistence,
  replay-proof, and reporting modules. Files over roughly 1,000 lines require an
  explicit single-responsibility justification or decomposition.
- [ ] Keep persistent corpora, checkpoints, journals, and models bounded,
  checksummed, atomic, and binary. JSON is limited to small human-facing
  manifests and summaries; prefer replay-and-record migrations over permanent
  compatibility complexity.

## Operating rules

- Work capability-first in the order above. Do not optimize throughput while a
  learning or execution semantic is known to be invalid.
- Every native campaign is bounded and answers multiple hypotheses. Never mine
  repeated campaigns for a lucky route.
- Every evaluated branch contributes authentic transitions and outcomes; every
  promoted route is reproduced by two cold replays.
- Use at most two owned build/native workers. Manage only exact child processes
  started by this session and never stop unrelated Codex, Cargo, emulator, or
  worker processes.
- Commit and push each natural milestone; do not leave a long-lived dirty tree.
