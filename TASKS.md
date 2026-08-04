# Usable route learning

## Objective

Build a generic learning system that discovers fast, creative routes through
native game state. It must learn useful actions, multi-action tactics, and
complete routes from experience; it must not depend on authored waypoints,
route-specific rewards, or a human route.

The immediate Ordon Springs problem is deliberately small: roughly four
seconds, or 120 frames at 30 FPS, from control to the real load zone. A serious
framework must solve this reliably and quickly before we claim it can handle
larger routing problems.

The known human route is 125 ticks. Required checkpoints are:

- first zero-shot route in minutes, not hours;
- a zero-shot route of 124 ticks or less;
- 123 ticks or less as evidence that the system can beat the baseline rather
  than merely reproduce it;
- 120 ticks or less as the current quality target.

A route counts only when the native load-zone predicate fires and two cold
replays reproduce its terminal identity and tick count. Human demonstrations
are optional training data for a later ablation, never a required input or a
policy ceiling.

## Current truth

- Native terminal detection, controller playback, cold replay, binary
  checkpoints, save states, branch orchestration, generic observations, and a
  generic action/option library already exist.
- The former scored scratch path incorrectly bypassed save-state branching and
  started every episode from the cold root. Its public command is retired;
  `tactic-route` is now the canonical zero-shot learner. Post-success scratch
  refinement still replays every complete candidate from the root and must not
  return as the primary learning path.
- The scratch learner found terminals, but its best ordinary learned route was
  hundreds of ticks long. Single-option heading, duration, roll, and deletion
  passes polished one lineage to 221 ticks. That is useful diagnostic evidence,
  not evidence of a capable learner.
- The 221 route remains 96 ticks slower than the human baseline. The exhausted
  single-option neighborhoods cannot express coordinated route rewrites such
  as turn, commit to a direction, and navigate around an obstacle.
- Recent refinement throughput was roughly 20 simulated native ticks per wall
  second and only tens of complete candidates per ten minutes. That is not
  useful exploration throughput.

Do not spend more native time completing or repeating the current
single-option coordinate-descent cycle. Preserve its artifacts as diagnostics
and move the learning path onto the existing save-state branch architecture.

## Non-negotiable design rules

- The learner optimizes authenticated terminal time. Position, velocity,
  facing, camera, collision response, action availability, and history are
  observations from which it may learn; they are not authored route rewards.
- Save states are working search nodes, not merely replay evidence.
- Every evaluated branch contributes transitions and outcomes. A failed branch
  cannot simply consume native time and disappear.
- Search must propose coordinated variable-length action sequences. Single
  edits may be retained as one proposal family, but cannot be the optimizer.
- Options and discovered tactics remain interruptible and state-conditioned.
- The learner may discover and promote tactics, but promotion must be justified
  by repeatable value or sample-efficiency gains, not an Ordon-specific name or
  coordinate.
- Persistent artifacts are bounded, checksummed, atomic binary formats.
- Run bounded experiments, retain distributions, and reject failed treatments.
- Use at most two owned native/build workers on this machine. Never stop or
  inspect unrelated Codex processes.
- Keep production code single-purpose and organized by subsystem. Split mixed
  execution, branching, learning, persistence, and reporting code instead of
  growing monoliths.
- Commit and push each completed milestone. Do not leave a long-lived dirty
  workspace.

## P0: reconnect learning to save-state branching

- [x] Map the existing save-state, branch scheduler, tactic-route, and scratch
  learner boundaries. Identify the exact bypass that forces scratch episodes
  and refinement candidates through cold-root execution. Record which existing
  components can be reused and which duplicate path should be retired.
- [x] Make an authenticated save state a first-class learner node. Each node
  must bind the native state fingerprint, best known root cost, observation,
  parent transition, and checkpoint identity.
- [x] Run alternative actions and short action sequences from the selected
  graph node. If no live handle survives, materialize that historical node once
  by authenticated prefix replay; sibling alternatives must restore the
  process-local checkpoint rather than replaying the prefix independently.
- [x] Prove deterministic restore semantics: identical state fingerprint,
  observation, action availability, continuation outcome, and terminal timing
  after repeated restores. Reject contaminated or process-local checkpoints.
- [x] Preserve every branch transition in the learning corpus, including
  collisions, stalls, reversals, nonterminal branches, and failed suffixes.
  Deduplicate by authenticated source state, action sequence, and successor.
- [x] Reconstruct any selected route through its parent transitions and cold
  replay it twice before promotion. Branch-local success is not final evidence.

Exit: one campaign explores a persistent graph of restored states and can
assemble and replay a terminal route without evaluating every proposal from
the root.

Architecture audit (2026-08-03):
`docs/route-learning-integration-audit.md` establishes `tactic-route` as the
canonical zero-shot learner and retires the separate cold-root `scratch-route`
entry point. The canonical path already owns the state graph, learned frontier
selection, sibling branching, transition admission, persistent worker fleet,
route reconstruction, and tactic mining. The measured implementation gap is
checkpoint locality for historical graph nodes: only the latest selected
endpoint has a coordinator-held process-local handle.

Existing graph verification (2026-08-03): 32 focused state-graph tests prove
exact authenticated nodes, root costs, parent expansions, terminal returns,
route reconstruction, counterfactual interior states, persistence, and
deduplication. Twelve worker-pool tests prove that an uncached historical node
is materialized before its action, sibling alternatives restore that source,
and the selected endpoint remains directly restorable. These are retained
canonical-runner capabilities, not new scratch-loop implementations.

Restore-locality audit (2026-08-03): the paired two-worker campaign at
`build/campaigns/ordon-p0-restore-locality-w2-d2-p2-r2-v4-20260803` passed both
repetitions with identical exploration-evidence digests and four useful graph
expansions per treatment. Owner-local continuation removed one prefix
materialization, 24 replayed prefix ticks, and 929,805-997,393 microseconds of
replay/restore work per pair. This audit also exposed and fixed a false fleet
constraint that bound reusable native processes to one complete learner-plan
hash even though every dispatched job and result already carries its own exact
plan authority.

Checkpoint parity audit (2026-08-03):
`build/benchmarks/ordon-p0-native-checkpoint-v2-20260803.json` passed at route
ticks 15, 62, and 124. Direct continuation and authenticated replay matched the
source state, complete transition, checkpoint-wide semantic digest, all 22
checkpoint-entry digests, terminal evidence bytes, and terminal boundary at
every frontier. The repeated restore-locality pairs above additionally matched
the applicable proposal surface and successor evidence. Invalid or foreign
process-local checkpoint identities remain fail-closed in the native result and
worker-session validators.

## P0: make branching fast enough to learn

- [ ] Benchmark existing cold-root replay, save, restore, short branch, and
  worker handoff separately. Report simulated ticks/second, branches/second,
  unique transitions/second, restore latency, CPU utilization, and bytes per
  retained node.
- [ ] Remove avoidable process boot, game boot, serialization, hashing, logging,
  and artifact-write work from the inner branch loop. Keep a native worker alive
  across many owned restores and branches where correctness permits.
- [x] Eliminate false rendering leakage at transition boundaries. The profiler
  previously timed the empty suppression branch, so an OS scheduling delay was
  mislabeled as renderer work. Suppressed samples now retain coverage while
  recording zero execution time; the sealed three-frontier checkpoint audit
  reports zero CPU renderer submission, audio emulation, and game-audio update
  time for every materialization, continuation, and replay batch.
- [x] Measure one versus two workers with isolated checkpoint ownership. Keep
  two only if end-to-end unique branch throughput improves without state
  contamination or host saturation.
- [x] Require restored suffix evaluation to beat equivalent cold-root candidate
  evaluation by at least 10x end to end. If it does not, treat save/restore
  performance or orchestration as broken and fix it before another route run.
- [ ] Establish a ten-minute capacity envelope showing how many unique decision
  states, short branches, transitions, and complete cold validations the local
  machine can produce. No multi-hour mining run is permitted to substitute for
  this gate.

Exit: the framework can evaluate thousands of meaningful short alternatives
within a ten-minute campaign and explains where every remaining wall-clock
second goes.

Checkpoint speed gate (2026-08-03): the parity-sealed direct one-tick
continuations took 336,013-368,567 microseconds end to end. Equivalent
authenticated prefix replay plus the same continuation took 3,823,491
microseconds at tick 15, 13,665,354 at tick 62, and 26,658,865 at tick 124:
approximately 10.4x, 37.1x, and 79.3x slower respectively. Native simulation
for the three direct continuations totaled 3,288 microseconds. These parity
batches intentionally spent 313,416-344,281 microseconds per tick on the
checkpoint-wide state proof; production tactic workers disable that proof under
the separately passed native-subsystem parity treatment. Campaign throughput
must therefore be measured on the production tactic path, not inferred from the
proof-enabled checkpoint audit.

Production throughput curve (2026-08-03): the bounded, parity-controlled curve
at `build/campaigns/ordon-p0-throughput-curve-w1-w2-d16-p2-r2-v1-20260803`
measured only 0.347 useful graph expansions/second with one worker and 0.369
with two. Native simulation consumed about two seconds of each 87-92 second
sample; goal-relabeled model updates consumed about 40 seconds. Two workers
improved end-to-end throughput by 6.3% with identical useful expansion evidence,
bounded memory, and bounded learner staleness, so the local limit remains two.

Achieved-goal relabel treatment (2026-08-03): uniformly sampled relabel targets
now grow with the square root of observed transitions instead of expanding the
early replay against every achieved endpoint. The otherwise identical passed
curve at `build/campaigns/ordon-p0-throughput-curve-w1-w2-d16-p2-r2-v2-20260803`
reduced one-worker median wall time from 92.29 to 63.06 seconds and two-worker
median wall time from 86.84 to 58.82 seconds. Model-update time fell from about
40 seconds to about 13 seconds per sample. Useful expansion throughput rose to
0.507/s and 0.544/s respectively; two workers now improve end-to-end throughput
by 7.2%. The treatment preserves exact reverse-path tick backups and passed the
existing around-corner, prompted-action, collision, trajectory, and achieved-
goal generalization controls. Its roughly 326 useful alternatives per ten
minutes remains far below the required thousands, so throughput is still a P0
failure rather than a solved gate.

## P0: learn trajectories and coordinated tactics

- [ ] Replace terminal-only whole-episode credit with state-graph backups over
  retained branch transitions. Terminal tick cost must propagate through
  predecessor states so experience at the corner changes choices before the
  corner on later searches.
- [ ] Select branch nodes using learned value plus uncertainty/coverage rather
  than repeatedly starting at the root or uniformly mutating the incumbent.
  Preserve an explicit exploration budget so early local estimates cannot
  permanently starve alternatives.
- [ ] Propose variable-length sequences from the generic action library,
  including simultaneous inputs and available prompted actions. The proposal
  mechanism must be able to change direction, camera/lock state, movement, and
  roll timing together when evidence supports it.
- [ ] Make action availability part of the state and proposal mask. The learner
  must know when roll, jump, mount, lift, interact, or future prompted actions
  are possible without receiving an authored instruction to use them.
- [ ] Retain trajectory-derived measurements such as displacement, velocity
  change, heading stability, collision response, and control continuity as
  observations. Demonstrate that the model can learn their relationship to
  eventual terminal time without hard-coded bonuses for straightness, rolling,
  wall contact, or any specific tactic.
- [ ] Mine repeated high-value action subsequences from successful branches.
  Promote a subsequence to a parameterized tactic only when replay across
  multiple compatible states improves value or sample efficiency over its
  primitive actions.
- [ ] Allow promoted tactics and primitive actions in the same policy. Tactics
  must compose, terminate early when their assumptions fail, and never prevent
  discovery of a better primitive or tactic sequence.
- [ ] Add deterministic tests in which the optimal solution requires a
  coordinated multi-action change separated by individually worse
  intermediates. The branch learner must cross this local optimum; the old
  single-edit optimizer should fail the control case.

Exit: accumulated branch experience discovers and reuses meaningful
multi-action behavior rather than only polishing a fixed action list.

## P0: prove Ordon is actually learnable

- [ ] Run five fixed zero-shot seeds under the ten-minute envelope. Report the
  full distribution of time to first terminal, terminals, unique states,
  branches, transitions, learned-choice changes, fastest ticks, and cold-replay
  results.
- [ ] Require all five seeds to reach the real load zone. Median time to first
  terminal must be measured in minutes, not hours.
- [ ] Inspect the learned state graph—not an authored route—to confirm that the
  system explored materially different approaches and learned a coordinated
  turn/navigation solution rather than inheriting the 221-tick lineage.
- [ ] Reach and cold-replay 124 ticks or less from a zero-shot campaign.
- [ ] Continue the unchanged generic framework to 123 ticks or less, then 120
  ticks or less. Beating 125 is evidence; merely approaching it is not success.
- [ ] Repeat with permuted seed order and equivalent budgets to prove the result
  is not scheduler ordering, lucky initialization, or update-stream leakage.

Exit: the generic learner reliably discovers substantially-better-than-human
Ordon routes within a useful local-machine time budget.

## P1: prove learning value and generality

- [ ] Compare adaptive branch selection against frozen-policy and random-valid
  controls over identical seeds, branch opportunities, and native budgets.
  Require a held-out gain in terminals per sample, time to first terminal, or
  terminal ticks.
- [ ] Run a separate ordinary-human-demonstration ablation. Measure whether it
  improves sample efficiency while allowing the learner to exceed the human
  route and discover tactics absent from the demonstration.
- [ ] Apply the unchanged framework, observations, action library, and tactic
  promotion rules to a second native route. Route-specific coordinates,
  rewards, and hand-authored tactics remain forbidden.
- [ ] Audit and split remaining mixed-responsibility production files. Keep
  execution, save states, branching, state graph, learning, tactic promotion,
  persistence, replay proof, and reporting independently testable.

Exit: learning causally outperforms controls and transfers to another route
without framework changes tailored to Ordon.
