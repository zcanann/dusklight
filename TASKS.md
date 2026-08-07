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

- Direct binary save-state restoration and non-root branching work. The last
  bounded zero-shot campaign performed 327 direct restores with no fallback.
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
  checkpoint and improves the 12-tick incumbent to 10 ticks. It then executes
  the complete 9-tick policy through a translated corridor and terminal whose
  exact state identities are disjoint from training but whose normalized
  observations are equivalent. This exposed and fixed scheduler admission of
  exhausted interior nodes; rollout-loop extraction and autonomous repetition
  across multiple environment variants remain open in P0 below.
- Native execution and the deterministic adequacy gate now share one online
  frontier selector, production parameterized proposal path, graph/policy lease
  authority, and batch-commit operation. The commit admits every executed
  alternative, keeps the pre-execution policy winner authoritative, backs up
  exact terminal cost, advances or freezes the policy as requested, projects
  authenticated terminal candidates, and reports the best graph terminal.
  Rollout-continuation cadence, horizon handling, and tactic promotion still
  live partly in the native adapter and remain the next extraction.

No task is blocked on design.

## P0 - make the production learner learn

- [ ] Extract one environment-independent learning/search loop from the native
  campaign. It must own frontier selection, action selection, rollout
  continuation, transition admission, terminal-cost backup, incumbent
  replacement, and tactic promotion through narrow interfaces. Native process,
  checkpoint, persistence, and reporting code remain adapters around that loop.
- [ ] Drive that exact loop with a deterministic checkpoint environment. The
  environment must include an around-corner local optimum in which greedy
  one-step goal progress fails and several temporarily worse actions are
  required. Starting with no learned return table, the production loop must:
  discover a terminal, branch from retained intermediate states, converge to
  the shortest route, and then repeat on exact-state-disjoint variants. A toy
  scheduler or an exhaustively pre-trained snapshot does not satisfy this task.
- [ ] Make successful rollouts teach multi-step terminal cost. Every transition
  on an authenticated terminal lineage receives exact ticks-to-terminal;
  repeated state/action evidence updates the policy used by later rollouts.
  Open rollouts remain censored rather than becoming fabricated failures, but
  their states, actions, availability, dynamics, and novelty remain usable
  exploration evidence.
- [ ] Make checkpoint rollouts the unit of optimization. Restore any useful
  incumbent or off-incumbent boundary, try alternative actions, and continue
  each candidate until terminal or a shared native-tick budget. Retain every
  intermediate boundary as future branch material. Immediately replace the
  incumbent when a faster authenticated terminal route appears.
- [ ] Keep both exploitation and discovery alive. Learned terminal cost ranks
  supported choices; uncertainty, state/action coverage, and seeded exploration
  allocate trials to unsupported choices. Neither a growing fresh-state queue
  nor repeated polishing of one lineage may starve the other partition.
- [ ] Mine repeated useful action subsequences into candidate tactics and test
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
