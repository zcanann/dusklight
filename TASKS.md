# Route-learning framework

## Mission

Build an algorithm-agnostic learning and search framework that can discover and
optimize native routes substantially beyond human play. Q-learning is one
candidate, not a requirement; use the method that learns best from the samples
and native throughput we can actually sustain.

The framework receives a native starting state, generic observations, the legal
action surface, and an authoritative terminal predicate. It must explore, learn
which behavior produced useful outcomes, turn discoveries into retained
policies or trajectories, minimize native completion cost, and emit an exactly
replayable input graph.

The action hierarchy must include all three sources:

- primitive controller inputs;
- reusable parameterized tactics supplied by us;
- tactics discovered, evaluated, promoted, composed, and retired by the system.

## ToOrdonSprings benchmark contract

Ordon is the first proving ground. The native load-zone predicate is the only
success authority, and native elapsed ticks are the score.

The headline result is **zero-shot**: the scored campaign starts from the named
root state without a human demonstration, authored route, waypoint sequence,
route-specific reward, proxy terminal, or tactic extracted from the human route.
It may use generic observations and a route-agnostic action/tactic library fixed
before the scored run. Human play may be evaluated later as optional off-policy
experience, but that is a separate treatment and cannot satisfy the zero-shot
claim.

The known human ToOrdonSprings route takes 125 ticks. Use this explicit ladder:

- **124 ticks:** first meaningful zero-shot human beat;
- **123 ticks:** stronger evidence that the framework can optimize beyond the
  baseline;
- **120 ticks or less:** clearly superior, TAS-grade headline result.

A score counts only when the framework's retained output reaches the native
load zone and cold-replays at least twice from the named root with identical
inputs, state identities, terminal proof, and tick count. A terminal reached by
an unselected coverage proposal is discovery evidence, not a scored route or
learning result.

## Work queue

These are the current missing capabilities, in execution order. Keep a thin
end-to-end campaign runnable, remove completed work from this file, and retain
detailed history in commits and sealed campaign artifacts.

### 1. Optimize selected zero-shot terminals

- [ ] Improve the retained 190-tick zero-shot route to 124 ticks or less using
  generic optimization. The automatic selector first converted the 231-tick
  archive winner into a minimized, twice-cold-replayed refinement incumbent.
  A bounded two-worker campaign against that promoted parent admitted 448
  candidates before its simulated-tick budget, improved 231 -> 226 -> 213, and
  automatically minimized, spliced, and reproduced the 213-tick winner from
  two fresh process boots with identical terminal boundary evidence. A second
  promoted campaign admitted 576 candidates, improved its 213-tick parent to a
  206-tick archive winner, and the automatic reducer removed one more tick;
  fresh process boots then reproduced the selected 205-tick route twice with
  identical boundary evidence. A third promoted campaign admitted 576
  candidates, improved 205 -> 201, and the reducer removed two additional
  ticks; fresh process boots reproduced the selected 199-tick route twice with
  identical tape and terminal boundary fingerprint. This proves selected-winner
  composition keeps improving its latest parent instead of repeatedly editing
  an obsolete tape. A fourth promoted campaign found 193 ticks after 128
  admitted candidates. A legal proposal later triggered a deterministic native
  message-font crash, but the sealed checkpoint finalizer selected, minimized,
  and reproduced the retained 193-tick route from two fresh process boots with
  identical tape and terminal boundary fingerprint. The winner therefore does
  not depend on completing or replaying the crashed generation. A fifth
  promoted campaign admitted 576 candidates before its simulated-tick budget,
  found four valid 191-tick committed-load candidates, and automatically
  minimized and reproduced the selected 191-tick route from two fresh process
  boots with identical tape and terminal boundary fingerprint. An apparent
  185-tick replay entry reached only the separate exit-approach predicate; the
  primary committed-load attempt missed, and selection correctly excluded it.
  The 213-tick intermediate changed only 32 stick-X frames and retained the
  same three roll presses, so these gains remain local residual progress rather
  than learned efficient movement. A later graph-scheduler correction exposed
  terminal alternatives across the machine-incumbent route and promoted a
  190-tick selected result. Two fresh process boots reproduced its identical
  controller tape, terminal boundary fingerprint, and 190-tick first hit. The
  route is still far from competitive.

Exit: each strict zero-shot improvement is automatically converted into a
selected, independently replayable route, and the retained route reaches the
authenticated terminal in 124 ticks or less.

### 2. Prove that accumulated experience improves future behavior

- [ ] Evaluate candidate learning/search algorithms against the real horizon,
  branching factor, transition volume, and update latency; do not preserve
  Q-learning by default. A ten-minute self-experience treatment from the
  authenticated 231 route evaluated 288 native alternatives over only 36
  decisions, consumed 58 imported route transitions, performed 19 learner
  updates, and retained no improvement. Its controlled policy-update probes
  never changed the selected action, so this treatment currently fails at
  policy effect. A ten-minute goal-relabeled fitted-Q follow-up from the
  retained 191 route evaluated 192 alternatives over 24 decisions, consumed 48
  machine-incumbent transitions, performed 13 learner updates, and again left
  all 11 valid same-state selected actions unchanged. It retained 191 ticks and
  spent 459.2 seconds fitting models, 3.4x the prior local-Q treatment's 135.9
  seconds. At this sample scale it is slower and still has no causal policy
  effect, so increasing its mining budget is not the next experiment. That
  campaign exposed a concrete policy-path defect: once any terminal evidence
  existed, the selector used only terminal-connected replay and stopped
  consulting the achieved-goal critic even though every open branch was still
  fitted into it. The terminal objective critic now owns only its dedicated
  exploitation partition; other partitions continue using the all-experience
  reachability critic, whose held-out calibration persists after terminal
  discovery. This policy-semantic change has a versioned learner snapshot and
  migrates older heads explicitly. A 16-decision, 128-proposal causal follow-up
  then completed in 91.6 seconds and exposed reachability evidence on 12
  post-terminal decisions. At its 128-row calibration prefix the model won only
  3 of 10 held-out action rankings (30.0% versus a 12.1% chance rate; 10.8%
  Wilson lower bound), so the gate correctly blocked all 12 deployments and no
  same-state action changed. The wiring and throughput are now adequate for a
  bounded larger-sample discriminator. That 64-decision treatment completed
  512 proposals in 352.2 seconds. At the 512-row prefix it won 22 of 58
  held-out action rankings (37.9% versus 12.1% chance; 26.6% Wilson lower
  bound), authorized reachability on 29 decisions, made it primary on 17, and
  changed one of 31 controlled same-state choices after a learner update. The
  treatment now proves policy effect, but retained 191 ticks. The first random
  control exposed an invalid comparison: non-learning policies received the
  48-transition demonstration in the replay corpus but not in their executable
  frontier graph. Assisted controls now import that common graph prior while
  still ignoring learned scores; unassisted random remains scratch. The
  corrected learned, frozen, and random-valid arms each ran 64 decisions, 512
  proposals, and exactly 560 authenticated graph expansions from the same
  demonstration. Learned produced 54 useful decisions versus 50 frozen and 40
  random, but every arm retained only the inherited 191-tick terminal. This is
  evidence that updates affect acquisition, not that they improve terminal
  outcomes. Their traces then exposed a scheduler defect common to every arm:
  `branch_every=4` and the four-way acquisition cycle were both indexed by the
  decision number, so every periodic graph branch landed on rank zero and all
  ranked exploration was starved. Graph acquisition now advances by branch
  episode, and assisted frontier coverage is scheduled directly rather than
  being throttled by the ordinary branch cadence. In the corrected matched
  arms, learned, frozen, and random all promoted 190 ticks. Random reached it
  at decision 14, frozen at 34, and learned at 57; learned consumed 11,808
  native ticks and 457.3 seconds versus 10,618/250.9 frozen and 11,209/267.6
  random. The scheduler repair improves search, but online learning still
  fails the terminal-outcome comparison. Do not extend this exact treatment
  without changing the learning or selection method. Inspection then found
  that V2's action-conditioned terminal Double-Q model selected actions but
  never informed frontier scheduling: equal-cost checkpoints on a successful
  route were ordered only by coverage. The explicit
  `goal_relabeled_frontier_double_q_v3` treatment now supplies learned
  state-action opportunity and uncertainty to those equal-cost choices while
  preserving V2 as a control. Synthetic campaign tests prove that only V3
  publishes the action-conditioned uncertainty needed to distinguish this
  estimate from V2's state-only value and that it causally precedes coverage.
  A matched native V2/V3/control treatment is still required before claiming
  improved terminal outcomes.
- [ ] Learn from successful and unsuccessful trajectories using sparse terminal
  value, authenticated tick cost, and generic state deltas.
- [ ] Support online collection, off-policy replay, prioritized reuse, temporal
  credit assignment, continued exploration, and escape from local optima.
- [ ] Surface motion history, velocity, orientation, camera, analog input,
  prompted-action availability, action duration, and resulting state deltas.
  Straight travel, velocity loss, collision slowdown, rolling, camera alignment,
  and detours must be learnable rather than encoded as Ordon-specific rewards.
- [ ] Run learned/adaptive, frozen-policy, and random-valid treatments over the
  same candidate stream, native budget, and campaign design.
- [ ] Require the adaptive treatment to reproduce terminals more often, consume
  fewer native samples, or achieve lower terminal ticks on held-out seeds.
- [ ] Verify that useful behavior survives ordinary seed ordering, campaign
  composition, and update-cadence changes.

Exit: retained experience causally changes future choices and improves terminal
outcomes relative to both controls. Merely finding another coverage terminal
does not pass.

### 3. Beat and then substantially exceed the human route zero-shot

- [ ] Produce and independently verify a zero-shot 124-tick route.
- [ ] Continue the same generic learning/search process to 123 ticks or less.
- [ ] Reach 120 ticks or less without adding an Ordon-specific route, reward,
  waypoint, exception, or hand-authored input sequence.
- [ ] Minimize the winning input graph without changing its terminal identity or
  native tick count.
- [ ] Retain a fixed-budget result distribution, not only the best lucky seed,
  so sample cost and reliability remain visible.
- [ ] After the zero-shot result exists, run a separate human-demonstration
  ablation to determine whether ordinary human play improves sample efficiency
  without becoming required or imposing a policy ceiling.

Checkpoint: 124 ticks is the first human beat; 123 ticks is stronger progress.

Exit: a zero-shot route reaches the native load zone in 120 ticks or less and
cold-replays twice with identical evidence.

### 4. Learn and compose tactics

- [ ] Represent primitives and tactics through one interruptible, parameterized
  option interface so either can be chosen at every valid boundary.
- [ ] Supply route-agnostic tactics such as moving toward a chosen heading,
  one-frame direction plus camera lock, camera lock plus action, rolling, and
  state-conditioned analog steering.
- [ ] Keep every currently legal primitive and prompted action available,
  including roll, jump, mount, lift, and future game-specific affordances.
- [ ] Mine useful sub-trajectories and repeated control structure; propose
  tactics with learned initiation conditions, parameters, termination
  conditions, and expected outcomes.
- [ ] Promote tactics only when held-out results improve over primitives and
  existing tactics; retire regressions and permit tactic composition.
- [ ] Keep primitive actions available so a bad abstraction cannot cap the
  policy.

Exit: supplied tactics improve search without encoding an Ordon route, at least
one automatically discovered tactic improves held-out performance, and tactic
composition exceeds the trajectories from which it was learned.

### 5. Scale throughput only when the learning experiment demands it

Optimize only costs demonstrated by the end-to-end learning and selection path;
do not golf isolated subphases without campaign evidence.

- [ ] Remove the measured learning-loop control-plane bottlenecks. In the
  36-decision self-experience treatment, native simulation occupied only 15.2
  seconds while graph scheduling and leasing took 179.8 seconds, generation
  replay and coordination 150.6 seconds, model updates 135.9 seconds, and
  persistence 148.8 seconds. Two-worker native utilization was only 8.4%.
  Live graph scheduling now earns one complete validation capability at
  checkpoint admission and reuses it after checked transactional mutations;
  graph-derived learning batches and rankings no longer repeat the same full
  traversal at every decision. External and reopened artifacts still validate
  fail-closed. The goal-relabeled follow-up measured only 24 decisions in 777.0
  seconds (32.4 seconds/decision versus 27.6 previously), with 8.35% native
  utilization. Its more expensive learner dominated the run, so the removed
  traversal did not improve end-to-end throughput under that treatment.
  After the critic-routing correction, the 64-decision causal treatment
  completed in 352.2 seconds (5.50 seconds/decision) with 9.92% native
  utilization and 124.8 seconds of model work. Its matched frozen and random
  controls completed in 214.0 and 215.2 seconds with only 19.7 and 17.6 seconds
  of model work. The end-to-end learning loop is inside the ten-minute gate at
  a sample count sufficient to adopt a policy, but online fitting adds roughly
  137 seconds without improving the terminal. Matched worker-scaling evidence
  is still required, and model cost should be reduced only alongside a changed
  treatment capable of improving outcomes. After repairing graph-acquisition
  starvation, the denser corrected schedule took 457.3 seconds learned versus
  250.9 frozen and 267.6 random. All three produced the same 190-tick best, so
  the learner's additional 152.9 seconds of model work remains pure overhead
  at this quality level.
- [ ] Cache or share already-validated artifact and graph identities within one
  process; independent reopen validation must remain fail-closed. Completed-seed
  preflight now carries the campaign-wide useful-expansion set directly from
  the authenticated graphs into final reporting instead of reopening every
  final checkpoint and decoding the same graph journals a second time. Newly
  sealed seeds additionally carry that projection across processes without
  retaining a live graph or weakening legacy checkpoint validation.
- [ ] Measure whether retained save-state branching beats replay from its
  authority point; simplify or remove it where it does not.
- [ ] Revisit deterministic generation-barrier or other concurrent learning
  schedules only after policy adoption and terminal optimization have causal
  tests.
- [ ] Eliminate measured worker starvation, redundant native work, unnecessary
  serialization, and central bottlenecks while keeping queues and memory
  bounded.

Exit: the next learning experiment, not an isolated microbenchmark, completes
within ten minutes; two workers materially improve unique accepted native
experience per second; retained save states have measured positive return.

### 6. Prove the framework is durable and general

- [ ] Apply the unchanged observation, action, learning, tactic, orchestration,
  and evaluation contracts to a second native route.
- [ ] Make fresh and resumed concurrent campaigns logically reproducible, with
  exact ownership for sampling, updates, model publication, checkpoints, and
  accepted experience.
- [ ] Make unrecoverable singleton native crashes explicit, bounded censored
  evidence. Residual batches now replace only their owned sessions and bisect
  failed work down to an exact candidate, and the legal proposal that exposed
  the headless message-font defect now completes under the repaired runtime.
  A singleton that still crashes must be durably charged and reported without
  fabricating a gameplay result or preventing selection of sealed evidence.
- [ ] Keep models, replay, checkpoints, and manifests compact, bounded, binary,
  versioned, checksummed, atomic, and migration-tested. The checked-in Windows
  launch-smoke bundle currently fails even against unmodified `a6893948a9`:
  checkpoint identity `77589090...a9919` reconstructs as
  `439e1d73...d70d`. Repair the migration/identity drift or reseal the fixture
  from authenticated evidence, then restore this check to the framework audit.
- [ ] Refactor oversized and mixed-responsibility code along observation,
  execution, state, replay, learning, tactics, workers, persistence, and
  reporting boundaries; enforce source-size and dependency gates.

Exit: the second route passes zero-shot discovery, learned improvement over
controls, tactic use, and exact cold replay without route-specific framework
changes; interruption and resume reproduce accepted-experience identities.

## Engineering rules

These apply while completing every queue item; they are not a later cleanup
phase.

- Campaigns collect one comprehensive reusable evidence stream. A run should
  answer many questions, including useful analyses not anticipated when it was
  launched. Rerun only for a changed treatment, genuine replication, or raw
  evidence that could not have been captured previously.
- Search retained benchmark artifacts and commit history before implementing a
  treatment; repeat a rejected intervention only when a named premise changed.
- Record observations, legal actions, choices, executions, transitions, replay
  admissions, updates, tactic decisions, terminals, timings, retries,
  rejections, duplicates, and censored work with stable identities.
- Throughput means unique accepted native experience per wall-clock second, not
  attempts, queued work, duplicate branches, or replayed frames.
- Score retained outputs, not the luckiest proposal observed anywhere in a
  batch. Report discovery, policy adoption, optimization, and final replay as
  separate outcomes.
- Own child processes directly. Cancellation, timeout, crash recovery, cleanup,
  progress, and resume must not scan for or affect unrelated processes.
- Use focused tests during iteration and broad suites at meaningful integration
  checkpoints. Do not substitute repeated testing for implementation progress.
- Benchmark gains count only when generic mechanisms, held-out evaluation,
  exact replay, and appropriate controls rule out route-specific gaming.
