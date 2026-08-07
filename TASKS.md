# Generic goal-learning framework

## Objective

Build a learner that uses native game experience to discover complete,
substantially better-than-human solutions to terminal game goals. The method is
not prescribed. Q learning, graph search, planning, imitation, and learned
options may be combined when the result learns reliably at the sample volume we
can actually execute.

The framework receives:

- a binary save-state root;
- a native terminal predicate;
- typed observations, including motion, collision response, input history, and
  currently available prompted actions;
- primitive, simultaneous, variable-duration, and promoted multi-action inputs;
- native tick cost.

It must not receive route coordinates, waypoints, route-specific tactics, or
hand-authored shaping rewards. Observations are evidence from which the learner
may infer useful behavior; they are not individually rewarded rules.

Ordon Springs is the first adequacy test. The real terminal is
`ordon_spring_load_committed`. The ordinary human route is 125 active ticks.
124 ticks is the first better-than-human gate, 123 is convincing evidence, and
120 is the current target.

## Authoritative current evidence

- The former native adapter remembered only its most recent live endpoint and
  silently root-replayed older logical branches. It now mirrors each worker's
  bounded portable-image LRU, returns newly materialized branch-base handles to
  the scheduler, and advances selected rollouts through a separate cheap live
  endpoint so progress cannot evict its own branch base. Focused dispatch tests
  cover multiple exact bindings, eviction, stale-handle fallback, and
  sibling-first live rearming. An end-to-end native search has not yet proved
  two nonconsecutive restores, so the checkpoint-rollout task remains open.
  Post-hoc inspection of the retained v4-v6 reports found 774 decisions labeled
  as process-local restores and zero whose source differed from the immediately
  preceding endpoint; the old count was entirely continuation reuse.
  A 96-decision width-one diagnostic against the corrected adapter selected six
  nonconsecutive branches, but all six still used authenticated root replay and
  zero used a nonconsecutive process-local restore. Ninety-four decisions used
  distinct frontier states: post-terminal scheduling rotated across exact
  interior states produced inside long options, while native retention covered
  only option endpoints or the branch currently being materialized. The next
  fix is scheduler-visible checkpoint locality and/or retention of the branch
  boundaries the scheduler actually chooses, not additional campaign volume.
  The controller now accepts bounded exact-state restoration preferences: a
  newly materialized portable base may source one additional rollout only when
  it remains nonterminal, within horizon, and has leaseable actions, after
  which the preference is consumed. Root-refresh authority and ordinary global
  acquisition remain unchanged. Native proof of this handoff remains open.
- The learner can discover the real terminal quickly, but its best authenticated
  route is 229 ticks. Two cold replays reproduced that route exactly.
- The first terminal arrives in roughly 35 seconds; hundreds of later
  expansions have not produced a competitive route. Discovery is no longer the
  primary failure. Route learning and optimization are.
- The present post-terminal loop learned one 254-tick refinement but did not
  beat the 229-tick incumbent. This is inadequate, not partial success toward
  the 125-tick gate.
- Existing paired-policy and causal-report machinery has zero valid native
  comparisons. Matched continuations are now explicit opt-in attribution mode;
  ordinary route learning no longer freezes its learner or spends a second
  full rollout on that control. Attribution remains off the critical path until
  an adaptive learner produces useful improvements worth explaining.
- The standalone around-corner fixture learns from an exhaustively enumerated
  training world and then transfers a completed return table. It does not run
  the production online campaign and therefore does not prove that the real
  learner can discover or improve a route.
- A production-path adequacy test now runs the real campaign, graph scheduler,
  retained-checkpoint restore, transition admission, terminal backup, and
  refit path online. It improves a deliberately selected 12-tick detour to a
  9-tick around-corner route without a test-authored root retry, publishes exact
  root returns of 10 versus 9, and subsequently ranks the 9-tick action first.
  Between those routes it autonomously restores an untried retained interior
  checkpoint and improves the 12-tick incumbent to 10 ticks. A second cold
  campaign independently converges in a translated corridor whose exact state
  identities are disjoint; its different exploration order finds 10 ticks on
  the first rollout and then 9, so the gate no longer enters the variant through
  a test-only transition or requires the same incidental route order. The first
  12-decision production rollout now
  remains coherent through the former eight-decision interruption point and
  runs to the terminal; ordinary rollouts branch only at a terminal, explicit
  curriculum/refinement boundary, or shared native-tick horizon. This exposed
  and fixed scheduler admission of exhausted interior nodes.
- Native execution and the deterministic adequacy gate now share one online
  frontier selector, production parameterized proposal path, graph/policy lease
  authority, and batch-commit operation. The commit admits every executed
  alternative, keeps the pre-execution policy winner authoritative, backs up
  exact terminal cost, advances or freezes the policy as requested, projects
  authenticated terminal candidates, and reports the best graph terminal.
  The shared continuation planner now also owns terminal, curriculum, and
  forced-horizon branching, terminal-refinement continuity, acquisition
  partitions, learned versus broad frontier use, root refresh, and exact
  frontier selection as one environment-independent operation. Native execution
  and the deterministic adequacy environment use the same controller to derive
  terminal restart/support, restore the returned branch, filter the
  state-specific applicable action surface, enforce the rollout horizon, and
  lease executable proposals. They do not recompose those decisions. There is
  no decision-count branch cadence. A stateful controller owns action ranking,
  continuation, restore, horizon-safe leasing, admission, and tactic status
  transitions; it records the outstanding decision index and refuses another
  cycle until that exact lease is admitted. Native process execution,
  persistence, timing, and reporting remain adapters around that controller.
- Terminal completion no longer rewrites every scheduled acquisition to rank
  zero. A terminal forces a restore while preserving the lane's sealed
  acquisition partition: rank zero exploits authenticated terminal paths and
  nonzero ranks continue broad graph discovery. Root refresh may replace a
  scheduled broad restore but cannot replace rank-zero terminal support. The
  production adequacy path now performs its 12-to-10-to-9 improvement through
  successive discovery ranks 1, 2, and 3, while the single-lane plan still
  reserves every fourth episode for exploitation.
- Tactic discovery no longer waits until campaign finalization. Between learned
  generations, the production runner mines the strongest repeated connected
  sequence from completed journals, validates it against its full primitive
  realization at matched held-out checkpoints, and adds a promoted guarded
  tactic to the next generation's live catalog without removing primitives.
  Checkpoint scheduling evaluates that same augmented action surface, so a
  compatible retained state becomes eligible again when its primitives are
  exhausted but the new tactic is untried.
  Validation now requires exact emitted-input equivalence. An equivalent
  multi-action sequence can promote for reducing several policy decisions to
  one; a single-action candidate still needs a strict native outcome gain.
  Frozen and random-valid controls do not receive this adaptive catalog update.
  The bounded active validator prioritizes true multi-decision compositions
  ahead of long one-component copies, so its single validation slot targets a
  candidate that can actually earn the decision-compression promotion.
  Promotion independence is defined by authenticated frontier identity, not by
  arbitrary seed identity: two distinct compatible source states and two
  distinct held-out validation states may come from one seed. This removes the
  former blanket ban on tactic promotion in a single-seed campaign without
  relaxing the multi-state evidence requirement.
  After a learned seed reaches a terminal, its durable journal is mined for the
  strongest repeated exact sequence. That sequence enters the same seed's live
  action catalog as an explicitly unpromoted candidate, survives resume through
  its checksummed binary registry, and can gather ordinary policy evidence while
  all primitive components remain selectable. Only the separate two-state
  matched candidate-versus-primitive gate changes its status to promoted.
  The production controller adequacy environment now promotes a two-input
  composition from matched evidence at two exact-state-disjoint compatible
  states, installs it beside every primitive, selects it through the ordinary
  leased decision path, and admits its transition. The promoted option emits
  the identical two-frame tape and reaches the identical exact state in one
  policy decision versus two primitive decisions.
  The checksummed macro lifecycle artifact now records active refresh count,
  newly promoted option identities, and later decisions that actually selected
  those options; these fields are derived from durable decision journals rather
  than inferred from catalog membership.
  Native evidence that a same-campaign promoted tactic was selected and
  improved learning remains open.
- Exact graph backup covers every executable node on every authenticated
  terminal tape, preserving route identity. The production adequacy gate now
  asserts the exact countdown on both complete 9-tick lineages and the 10-tick
  retained-interior shortcut. Separate production graph/learner coverage proves
  open continuations remain right-censored while still teaching duration,
  acceptance, prompted-action availability, next-state dynamics, uncertainty,
  and prediction error.

No task is blocked on design.

## P0 - make the production learner learn

- [x] Extract one environment-independent learning/search loop from the native
  campaign. It must own frontier selection, action selection, rollout
  continuation, transition admission, terminal-cost backup, incumbent
  replacement, and tactic promotion through narrow interfaces. Native process,
  checkpoint, persistence, and reporting code remain adapters around that loop.
- [x] Drive that exact loop with a deterministic checkpoint environment. The
  environment must include an around-corner local optimum in which greedy
  one-step goal progress fails and several temporarily worse actions are
  required. Starting with no learned return table, the production loop must:
  discover a terminal, branch from retained intermediate states, converge to
  the shortest route, and then repeat on exact-state-disjoint variants. A toy
  scheduler or an exhaustively pre-trained snapshot does not satisfy this task.
- [x] Make successful rollouts teach multi-step terminal cost. Every transition
  on an authenticated terminal lineage receives exact ticks-to-terminal;
  repeated state/action evidence updates the policy used by later rollouts.
  Open rollouts remain censored rather than becoming fabricated failures, but
  their states, actions, availability, dynamics, and novelty remain usable
  exploration evidence.
- [ ] Make native checkpoint rollouts the unit of optimization. The logical
  graph and native adapter now retain bounded, eviction-aware restore handles
  attached to exact graph states while keeping the selected live endpoint
  separate. Prove the production scheduler actually restores at least two
  nonconsecutive intermediate boundaries without root replay and improves an
  incumbent from one of them. Continue each candidate until terminal or a
  shared native-tick budget, and immediately replace the incumbent when a faster
  authenticated terminal route appears.
- [x] Keep both exploitation and discovery alive. Learned terminal cost ranks
  supported choices; uncertainty, state/action coverage, and seeded exploration
  allocate trials to unsupported choices. Neither a growing fresh-state queue
  nor repeated polishing of one lineage may starve the other partition.
- [x] Mine repeated useful action subsequences into candidate tactics and test
  them against their primitive realization from matched checkpoints. Promote
  only when they improve terminal rate, terminal ticks, or sample efficiency
  across multiple compatible states. Primitive actions must remain selectable.

Exit: the production learning/search loop, without native-specific knowledge,
learns shortest solutions in deterministic checkpoint worlds from online
experience and demonstrates that later choices changed because of learned
multi-step return.

## P0 - beat the Ordon route

- [ ] Run one bounded seed-104729 campaign through the corrected loop. It must
  report, in one compact scorecard, time to first terminal, incumbent
  improvements, branch sources, rollout completions, censored rollouts,
  save-state restores, action/tactic coverage, useful experience per second,
  and final cold-replay result. If it cannot improve 229 ticks, diagnose and fix
  the learning/search decision that prevented improvement before adding volume.
- [ ] Discover and cold-replay a zero-shot route of 124 ticks or less twice,
  with identical terminal identity and first-hit tick.
- [ ] Reach 123 ticks or less and then 120 ticks or less with the same generic
  observations, objective, actions, and learning rules.
- [ ] Run five fixed zero-shot seeds under the same bounded envelope. All five
  must reach the native terminal, the median discovery time must be useful on a
  local machine, and the route-quality distribution must not depend on one
  lucky seed.
- [ ] Repeat with permuted seed order and equivalent budgets to rule out update
  ordering, scheduler ordering, and cross-run leakage.

Exit: the framework reliably discovers substantially better-than-human Ordon
routes; merely approaching 125 ticks does not pass.

## P0 - prove learning and tactics caused the result

- [ ] After the adaptive learner improves a route, compare it with frozen-policy
  and seeded random-valid controls over identical checkpoints, opportunities,
  seeds, and native-tick budgets. Reuse retained evidence when identities match;
  collect new paired evidence only where outcomes are censored. Require a
  held-out gain in terminal rate, time to first terminal, route ticks, or useful
  expansions to the result.
- [ ] At decisive checkpoints, show that experience changed action ranking and
  rollout selection. Audit learned dependence on generic observations such as
  velocity, trajectory continuity, momentum loss, collision response, camera
  and input history, and prompted-action availability. Do not convert those
  observations into authored reward bonuses.
- [ ] Show at least one discovered and promoted multi-action tactic improving
  held-out native decisions relative to its primitive components.
- [ ] Run a suboptimal human-demonstration ablation. A demonstration may improve
  exploration or representation learning, but the learner must still succeed
  without it, exceed it, and discover tactics absent from it.

Exit: adaptive learning and reusable tactics, rather than lucky search or a
hand-authored route, causally improve the result.

## P0 - make useful learning fast enough

- [ ] Profile the corrected end-to-end loop once under a representative growing
  corpus. Attribute wall time and memory across native execution, restore,
  model update, graph scheduling, evidence admission, persistence, and IPC in
  the same run.
- [ ] Optimize only measured dominant costs. Eliminate repeated whole-corpus
  projection, hashing, fitting, or serialization when retained incremental
  state can preserve the same semantics. Do not weaken exploration or evidence
  integrity to inflate throughput.
- [ ] Re-run the same treatment and require a material increase in useful
  experience per minute or a material reduction in time to the same learned
  result. A microbenchmark without end-to-end movement does not pass.
- [ ] Compare one and two checkpoint-owning lanes. Keep added parallelism only
  when it increases unique useful experience without learner contamination,
  host saturation, or interference with unrelated processes.

Exit: the framework explains where its time goes and reaches the same learning
quality materially faster.

## P1 - generality and engineering quality

- [ ] Apply the unchanged objective interface, observations, action library,
  learning rules, and tactic promotion to a second native goal. No
  route-specific coordinates, shaping, or named tactic may be introduced.
- [ ] Split mixed-responsibility production files as the learning loop is
  extracted. Execution, checkpoint ownership, branching, learning, proposal
  generation, tactic promotion, persistence, replay proof, and reporting must
  be independently testable modules. Production files over roughly 1,000 lines
  require an explicit single-responsibility justification or decomposition.
- [ ] Keep large persistent corpora, checkpoints, journals, and models bounded,
  checksummed, atomic, and binary. JSON is acceptable only for small human-facing
  manifests and summaries. Prefer an explicit replay-and-record migration over
  permanent compatibility complexity.

Exit: the learner transfers beyond Ordon and its critical behavior can be
audited, tested, and changed without navigating orchestration monoliths.

## Operating rules

- Work in the order above unless retained evidence proves a later task is the
  current blocker.
- Every native campaign is bounded and answers multiple hypotheses. Do not run
  repeated campaigns merely to mine a better random result.
- Tests and reports are evidence only when they exercise the production path
  named by the task.
- Every evaluated branch contributes its authentic transitions and outcomes.
- Every promoted route is reproduced by two cold replays.
- Use at most two owned build/native workers on this machine. Manage only exact
  child processes started by this session; never stop unrelated Codex, Cargo,
  emulator, or worker processes.
- Commit and push each natural milestone. Do not leave a long-lived dirty tree.
