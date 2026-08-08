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

- The production online loop learns the 9-tick shortest route in two
  exact-state-disjoint deterministic around-corner worlds. It restores retained
  states, propagates exact terminal cost through temporarily worse moves, and
  keeps censored rollouts as dynamics and coverage evidence without assigning
  them a terminal return.
- Native process-local checkpoints are active optimization state, not passive
  artifacts. A matched run restored nonconsecutive prefixes directly and
  improved an authenticated incumbent from 274 to 251 ticks. Forced
  perturbations, coordinate rejoins, and privileged incumbent continuations are
  absent; subsequent actions come from the ordinary learner.
- The state-aware action surface exposes movement, roll, prompted actions,
  camera lock, simultaneous inputs, and variable durations. Native prompt state
  controls legality. The V4 goal-relabeled universal frontier learner consumes
  generic motion, collision, camera, input-history, and availability features;
  none is converted into a route-specific reward bonus.
- The learned-option loop is now end to end. Connected terminal graph lineages
  produce candidates; matched native execution compares each candidate with its
  exact primitive components at two held-out states; only promoted candidates
  enter the ordinary action surface. Authentic but non-composable replay remains
  learner evidence and is counted rather than aborting mining. A sealed
  four-generation, zero-demonstration campaign admitted 11 of 14 promoted-option
  transition rows, activated four promoted options, and made three later
  ordinary selections. Two were selected by learned generalized value. Each
  selected option contained two validated primitive components, saving three
  policy decisions while the primitive actions remained available.
- That same bounded campaign reached the real terminal in all four seeds and
  improved its authenticated incumbent from 403 to 230 to 222 ticks. It used
  direct process-local restores in every seed and sealed its report, summary,
  learner completion, and campaign completion. This is useful framework
  evidence, but 222 is still nowhere near the 124-tick adequacy gate. The older
  210-tick artifact remains the best cold-replayed route and is not directly
  comparable to this treatment.
- Its repaired v7 audit distinguishes inherited campaign incumbents from local
  discoveries. The scorecard exposed the first policy failure: 94 of 128
  retained decisions used exact-ID `unsupported_bootstrap`, only two retained
  actions had an exact fitted value, and just one of 26 policy-update probes
  changed the selected action. A generalized model candidate was present in
  105 batches but controlled only 27 decisions because uncalibrated adaptive
  deployment was arbitrarily limited to every fourth decision. The matched
  consecutive-control run moved generalized-value control to 108 of 128
  decisions and changed three of 25 policy-update probes, but regressed from
  222 to 278 ticks. Its terminal critic had zero held-out predictions or
  comparable state groups, learned only from 16 terminal-connected rows, and
  collapsed toward long controllers. Terminal samples no longer authorize that
  critic: the all-experience achieved-goal critic retains authority until
  terminal ranking beats chance on held-out same-state siblings.
- The full orchestration suite passes: 494 tests, including exact-return policy
  adoption, learned-option decision compression, checkpoint branching, replay
  recovery, and campaign report validation.

No task is blocked on design.

## P0 — make native checkpoint learning coherent

- [x] Make retained checkpoints the actual unit of optimization. Demonstrate in
  the production loop that at least two nonconsecutive intermediate states are
  restored without root replay and that ordinary subsequent decisions from one
  produce a faster authenticated terminal route.
- [x] Verify that terminal returns change later action ranking at predecessor
  states and propagate far enough to solve temporarily-worse moves around a
  corner. Open rollouts remain censored but still teach availability, dynamics,
  duration, novelty, and uncertainty.
- [x] Keep exploitation and discovery live without decision-count scripts or
  forced route phases. Learned terminal value should revisit promising prefixes;
  uncertainty and state/action coverage should allocate unsupported trials.
- [x] Expose simultaneous and variable-duration primitives—including movement,
  camera lock, roll, and their legal compositions—through the same state-aware
  action interface. Availability must come from native state, not a route script.
- [x] Complete the learned-option loop: mine useful connected subsequences,
  validate them against their primitive realization from matched retained
  states, promote only improvements that generalize across compatible states,
  and give the ordinary policy evidence with which to rank and select the
  promoted option in a later decision. Demonstrate a real selection that saves
  policy decisions or native route ticks. Primitives remain available.

Exit: the production learner, without a privileged route continuation, learns
the shortest solutions in deterministic checkpoint worlds and improves a native
incumbent from a nonconsecutive restored checkpoint.

## P0 — beat the Ordon route

- [x] After the coherence exit above, run one bounded seed-104729 diagnostic that
  answers multiple hypotheses in one scorecard: terminal discoveries, incumbent
  improvements, branch sources, completed and censored rollouts, restore modes,
  action/tactic coverage, learned ranking changes, useful experience per second,
  and cold-replay result. The four-generation seed-104729 campaign produced 260
  completed executable expansions (1.69/s), 249 native transitions of which 144
  were censored, 130 direct restores with zero fallback replays, and local
  incumbent discoveries of 403, 230, and 222 ticks. The 222 route cold-replayed
  twice with identical terminal identity and first-hit tick. Its first
  post-terminal failure was decision 17: an evidence-backed generalized
  candidate was evaluated, but the periodic gate retained an exact-ID-unknown
  action instead. Diagnose first, then change the policy; do not add volume.
- [x] Re-run that bounded treatment once with consecutive adaptive model control.
  The audit proved that the learned selector controlled most decisions and that
  updates changed ranking more often, but the 278-tick regression exposed an
  uncalibrated terminal-only critic rather than a volume problem. Terminal
  samples alone can no longer hand off action or checkpoint-frontier authority.
  An uncalibrated terminal critic may collect a distinct sibling probe while
  the achieved-goal critic continues learning from every completed and censored
  branch.
- [ ] Re-run the same bounded treatment once with calibration-gated terminal
  authority. Before deployment readiness, require zero terminal-critic primary
  selections, achieved-goal control whenever its evidence gate passes, and
  terminal probes only as siblings. If it does not improve 222 ticks, diagnose
  the first wrong retained decision from this run rather than adding volume.
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
