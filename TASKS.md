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
- Every non-root decision must branch from one rewindable binary machine
  snapshot. A process-local live endpoint may accelerate one continuation, but
  it is single-use and is not a substitute for the save state needed to restore
  sibling alternatives.
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
- [x] Enforce the non-root branch invariant in the worker scheduler: materialize
  at most one rewindable source snapshot for a decision, execute compatible
  siblings in one multi-candidate restore batch, and retain only the selected
  child as the next graph-node snapshot. Never run sibling candidates from the
  cold root merely because a process-local handle is unavailable.
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
- [ ] Make rewindable snapshot capture and restore cheap enough for the branch
  cadence. Profile machine-image capture, host-state capture, cache insertion,
  restore, and eviction independently; optimize the measured representation or
  memory-copy path rather than replacing rewindable branch state with a
  single-use live endpoint.
- [ ] Remove avoidable process boot, game boot, serialization, hashing, logging,
  and artifact-write work from the inner branch loop. Keep a native worker alive
  across many owned restores and branches where correctness permits.
- [x] Execute sibling alternatives from one graph-node checkpoint in one native
  multi-candidate batch, or schedule independent graph nodes concurrently. A
  proposal-width-two decision must not choose between duplicating the entire
  prefix on another process and serializing two separate native requests on one
  process. Prove the chosen design against an optimized-orchestrator control.
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

Debug-orchestrator throughput curve (2026-08-03): the bounded, parity-controlled curve
at `build/campaigns/ordon-p0-throughput-curve-w1-w2-d16-p2-r2-v1-20260803`
measured only 0.347 useful graph expansions/second with one worker and 0.369
with two. Native simulation consumed about two seconds of each 87-92 second
sample; goal-relabeled model updates consumed about 40 seconds. Two workers
improved end-to-end throughput by 6.3% with identical useful expansion evidence,
bounded memory, and bounded learner staleness. The report's recorded orchestrator
hash is the exact hash of `target/debug/huntctl.exe`; it is not valid production
throughput evidence.

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
minutes was also measured through that debug orchestrator and must not be used
as the local capacity estimate.

Optimized control (2026-08-03): the exact branch-cadence-four plan was rerun
through the pre-change optimized release binary at
`build/campaigns/ordon-p0-throughput-optimized-control-w1-w2-d16-p2-r2-v1-20260803`.
It measured 0.949 useful expansions/second with one worker and 1.426/s with two;
two workers achieved 1.50x speedup and 75.1% parallel efficiency. That projects
to roughly 856 useful alternatives per ten minutes, still below the required
thousands but materially different from the invalid debug-binary estimate.

Checkpoint-locality treatment (2026-08-03): an exact-plan experiment at
`build/campaigns/ordon-p0-throughput-portable-owner-w1-w2-d16-p2-r2-v2-20260803`
retained selected nodes as reusable portable images and kept each narrow sibling
batch on its owner. In the two-worker sample it reduced prefix materializations
from 18 to 3 and replayed prefix ticks from 1,206 to 148 while preserving the
same state-graph identity and 32 useful expansions. It did not improve optimized
wall time: the median changed from 22.44 to 23.10 seconds because the two sibling
requests became serial. The experiment was reverted because locality alone is
not a throughput win when it removes proposal parallelism.

Native sibling-batch treatment (2026-08-03): compatible static, compiled native-
generic, and cancellation-free reactive siblings now execute in one compact
native request from the same restored graph-node checkpoint. Candidate horizons
remain independent, and only the selected child is retained as a reusable
portable image. The same-native, same-plan, same-`campaign`-profile A/B compares
`build/campaigns/ordon-p0-throughput-portable-owner-same-native-control-w1-w2-d16-p2-r2-v1-20260803`
against
`build/campaigns/ordon-p0-throughput-native-sibling-batch-campaign-profile-w1-w2-d16-p2-r2-v1-20260803`.
Both produced the identical state graph
`578d53b8c7faffdf410d4ed7349aa1f5f6d4ad8c3e4c63496816fed9044e4ede`,
identical useful-expansion set
`a22b08debae51e07a04e49ca20e944ef7b125385f60724383a2181d0a3caef13`,
and 32 useful expansions per sample. With one worker, median wall time fell from
23.06 to 16.06 seconds and throughput rose from 1.388 to 1.993 useful
expansions/second (30.4% less wall time, 43.6% more throughput). Prefix
materializations fell from 15 to 3, replayed prefix ticks from 1,058 to 148, and
non-root restore requests from 30 to 15. With two workers, wall time fell from
16.54 to 16.00 seconds and throughput rose from 1.934 to 2.000/s. The bounded
capacity projection is therefore about 1,196 useful alternatives per ten
minutes. A second worker provides no material single-lane benefit after sibling
batching; additional worker throughput must come from independent graph nodes or
concurrent lanes, not from duplicating one decision's prefix.

Branch-state invariant (2026-08-03): the selected portable image is intentional,
not serialization fallback. A live endpoint contains enough host authority to
continue once but cannot rewind the emulated machine after the first sibling
advances it. Replacing the selected image with live-only retention would either
fail the second sibling or force another authenticated prefix replay. The hot
path therefore captures one binary node image per selected decision and restores
all compatible siblings from it; future work must reduce that single capture's
measured cost or introduce an equally rewindable copy-on-write representation.

Checkpoint-image reuse treatment (2026-08-03): retained nodes are 295,040,780-
byte binary images. The cache previously evicted an unpinned image and then
allocated an equally sized replacement for the selected child. It now transfers
the deterministic LRU victim's exact-manifest buffers into the fresh capture
while leaving the source pinned. The same instrumented executable exposes a
disabled control through `DUSKLIGHT_DISABLE_CHECKPOINT_IMAGE_REUSE=1` and reports
reuse attempts/successes in every native result. Across the eight fixed-work
samples in the two control and two treatment curves under
`build/campaigns/ordon-p0-throughput-checkpoint-image-reuse-same-native-*`, the
median checkpoint-capture total fell from 3,184,173 to 2,203,433 microseconds
(30.8%), native tactic execution fell from 7,475,800 to 6,504,213 microseconds
(13.0%), and end-to-end wall fell from 10,986,809 to 10,202,695 microseconds
(7.1%). All samples produced graph
`a960bb9987e48e87e473481f0d3725bfb7f628cb22368c40de5e5fd336207d31`
and useful-expansion set
`8ec759960082b003100550f4b0f4e1db9540a33c43d448cb23767f06cb932402`.
The resulting local projection is about 1,882 useful alternatives per ten
minutes: a real improvement, but still below the thousands gate. Persistence
remains approximately 1.6 seconds in uncontended samples and produced several
multi-second outliers, so it is the next independent throughput target.

Packed-transition persistence treatment (2026-08-03): every completed proposal
was previously persisted as separate before/after facts, tactic metadata,
emitted input, intermediate facts, and a transition manifest, with a durable
file sync for each immutable object. The writer now stores one checksummed CBOR
transition object; the reader retains and tests the former split-object format
so existing checkpoints remain readable. Against the stable checkpoint-reuse
v2 curve with the same executable binding, execution plan, 16 decisions, and 32
useful expansions, median replay-content persistence fell from 717,708 to
202,650 microseconds (71.8%) and median total persistence fell from 1,559,772 to
985,340 microseconds (36.8%). One sample's content store fell from 298 to 123
objects (58.7% fewer); packed values increased its total bytes from 60,425,108
to 61,697,547 (2.1%), an accepted bounded tradeoff for eliminating file-sync
fan-out. The fixed-work curve at
`build/campaigns/ordon-p0-throughput-packed-transition-w1-w2-d16-p2-r2-v1-20260803`
measured 3.419 useful expansions/second with one worker and 3.497 with two,
projecting about 2,098 useful alternatives per ten minutes. Every sample
retained graph
`a960bb9987e48e87e473481f0d3725bfb7f628cb22368c40de5e5fd336207d31`
and useful-expansion set
`8ec759960082b003100550f4b0f4e1db9540a33c43d448cb23767f06cb932402`.
This crosses the raw thousands-throughput gate; the capacity-envelope task
remains open until it also accounts for complete cold validations.

## P0: learn trajectories and coordinated tactics

- [x] Replace terminal-only whole-episode credit with state-graph backups over
  retained branch transitions. Terminal tick cost must propagate through
  predecessor states so experience at the corner changes choices before the
  corner on later searches.
- [x] Select branch nodes using learned value plus uncertainty/coverage rather
  than repeatedly starting at the root or uniformly mutating the incumbent.
  Preserve an explicit exploration budget so early local estimates cannot
  permanently starve alternatives.
- [x] Propose variable-length sequences from the generic action library,
  including simultaneous inputs and available prompted actions. The proposal
  mechanism must be able to change direction, camera/lock state, movement, and
  roll timing together when evidence supports it.
- [x] Make action availability part of the state and proposal mask. The learner
  must know when roll, jump, mount, lift, interact, or future prompted actions
  are possible without receiving an authored instruction to use them.
- [x] Retain trajectory-derived measurements such as displacement, velocity
  change, heading stability, collision response, and control continuity as
  observations. Demonstrate that the model can learn their relationship to
  eventual terminal time without hard-coded bonuses for straightness, rolling,
  wall contact, or any specific tactic.
- [x] Mine repeated high-value action subsequences from successful branches.
  Promote a subsequence to a parameterized tactic only when replay across
  multiple compatible states improves value or sample efficiency over its
  primitive actions.
- [ ] Allow promoted tactics and primitive actions in the same policy. Tactics
  must compose, terminate early when their assumptions fail, and never prevent
  discovery of a better primitive or tactic sequence.
- [x] Add deterministic tests in which the optimal solution requires a
  coordinated multi-action change separated by individually worse
  intermediates. The branch learner must cross this local optimum; the old
  single-edit optimizer should fail the control case.

Exit: accumulated branch experience discovers and reuses meaningful
multi-action behavior rather than only polishing a fixed action list.

Graph-learning integration checkpoint (2026-08-03): `StateGraph` computes exact
authenticated ticks-to-go for every executable predecessor on a terminal tape;
`GraphLearningBatch` projects each retained expansion as either an exact
terminal-connected target or a censored open continuation, and the canonical
action scheduler fits and ranks from that graph evidence. The runner previously
failed to use its pre-terminal goal-reachability model when choosing a retained
checkpoint: ordinary discovery went directly to coverage/novelty rotation even
after fitting the learned scorer. Pre-terminal goal-relabeled campaigns now use
the learned frontier acquisition, with expansion coverage and uncertainty in
the ranking. After terminal discovery, rank zero owns value optimization while
the remaining acquisition ranks stay on broad graph exploration; periodic root
refresh remains an independent sealed cadence. Deterministic scheduling,
goal-reachability ranking, exact predecessor-return, censored-continuation, and
restart tests pass in the 449-test orchestration suite.

Coordinated-action checkpoint (2026-08-03): the canonical family generator
offers 4/8/16/40-tick movement, curved movement, camera-lock setup, directional
rolls, and combined, one-frame-staggered, and fully staggered direction/L/A
programs. Native `do_status` facts drive both prompt features and the applicable
action mask. The fixed observation schema retains displacement, velocity,
heading/goal alignment, commanded stalls, momentum loss, collision correction,
wall contact, and prior control; the route reward remains only authenticated
terminal success minus native tick cost. Replay mining covers both repeated
option prefixes and connected multi-option sequences. Promotion requires native
comparison against the original primitive-component sequence at two distinct
held-out states and seeds, and imported promoted tapes join the same catalog
without removing primitive families. Mid-option assumption failure is not yet
an executable cancellation guard for recorded macros, so that task remains
open. The around-corner fixture now explicitly proves that the learned held-out
route commits to three initially worse moves while a one-step goal-progress
control cycles and fails.

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
