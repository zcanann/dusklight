# Active tasks: make route learning auditable, scalable, and effective

This file contains only unfinished learning-framework work. Completed
implementation belongs in Git; benchmark history belongs in immutable reports.

## Product objective

Build a generic checkpointable optimizer that:

1. starts from an authenticated native checkpoint;
2. observes typed game state, prompted-action availability, and measured
   trajectories;
3. proposes primitive controller actions and learned action compositions;
4. evaluates alternatives from retained native states;
5. shares every authenticated transition with a state/action learner;
6. discovers and validates reusable tactics; and
7. minimizes native input ticks to an authenticated terminal.

The learner receives observations, available actions, and terminal/tick return.
Trajectory, velocity, collision, straightness, rolling, and prompted-action
availability may be inputs or auxiliary prediction targets, but are not
hand-authored utility.

A human recording is optional, ablatable replay. It is not privileged policy,
state, reward, or terminal authority.

This is not a TAS-authoring or graph-UX backlog. Graph recording/playback and
the interactive route planner are separate products. Tactics here are learned
runtime action compositions, never UI-authored blueprints.

## Current decision state

Ordon is the acceptance benchmark. The eligible request starts at authenticated
boundary `506`; the human incumbent first reaches the actual load-zone terminal
at tick `125`. The best machine result ties `125`; it has not beaten it.

- Shared replay is no longer the immediate bottleneck. The fixed 1/2/4-worker
  comparison passed with one campaign-owned fitter, immutable snapshots, and
  zero lane-local fits. Evidence:
  `ordon-p1-online-replay-scaling-v2.report.json`.
- Selected route endpoints now remain as bounded process-local continuations
  instead of copying a roughly 295 MB machine image on the decision hot path.
  The representative early/middle/late benchmark proved exact continuation
  versus authenticated-replay parity with 296-byte live endpoints and zero
  machine capture. An ordinary width-4/four-worker Ordon campaign kept all 32
  proposal transitions, used live continuation for all seven warm selected
  follow-ups, captured no portable checkpoints, and measured orchestration
  plus persistence at 3% of native simulation. Evidence:
  `ordon-p2-live-checkpoint-path-v1.report.json` and
  `ordon-p2-live-fixed-route-v1.report.json`.
- Suppressing presentation work did not materially improve proof-mode wall
  time. State proof and checkpoint work dominate, so further speculative
  renderer stripping is not the next priority. Evidence:
  `ordon-p2-headless-path-v1.report.json`.
- Live learned decisions now consume newly fitted snapshots, preserve exact
  greedy/epsilon authority, expose prompted-action factors, and can learn and
  repeat a terminal action. The resulting suffix is still `137` ticks against
  a deliberately degraded `131`-tick demonstration, so delayed-credit route
  improvement remains unproved. Evidence:
  `ordon-p3-greedy-authority-v1.report.json`.
- The continuous fitted-Q forest remains a sealed explicit control, but is no
  longer the live default. Independent semantic-group calibration found higher
  weighted error than the local generalized fitted-Q k-NN treatment, and
  positive demonstration replay caused unsupported actions to receive nearly
  terminal-quality estimates. The local treatment is again the default while
  native validation continues. Evidence:
  `ordon-p3-control-comparison-v1.report.json` and
  `ordon-p3-continuous-independent-calibration-v1.report.json`.
- The current continuous treatment completed matched scratch,
  ordinary-demonstration, and structured non-learning native cells under the
  same fixed identities and budget caps. Scratch and structured exploration
  found no terminal. The demonstration-assisted cell retained two authenticated
  terminal candidates at tick `127`, but the v2 summary incorrectly counted
  only candidates beating the tick-`125` promotion threshold and therefore
  reported no terminal. Route reports now expose terminal discovery, best
  authenticated tick, and promotion success separately. The cell still failed:
  it used the tick-`125` incumbent rather than an ordinary suboptimal sample,
  was slower than scratch, and did not improve on the replay.
  Evidence: `ordon-p3-continuous-matched-controls-v2.report.json` and its
  correction `ordon-p3-continuous-matched-controls-v3.report.json`.
- Four independent scratch seeds produced no terminal. Across disjoint
  semantic groups, continuous fitted-Q won `8/10` state orderings and `11/16`
  action-realization orderings; the structured shortest-valid-action control
  won `10/10` and `15/16` with lower weighted MAE on both axes. Useful native
  evidence arrived at `0.471` transitions per wall second. The treatment has
  signal but is not robustly superior, and terminal-value learning remains
  unmeasured because no scratch seed succeeded. Evidence:
  `ordon-p3-continuous-independent-calibration-v1.report.json`.
- The support-aware local treatment now has a sealed ablation against an
  ordinary `131`-tick human replay. At the full matched budget, replay-assisted
  learning found authenticated terminals in `3/4` seeds while scratch found
  `0/4`; useful training evidence per wall second improved by `21%` and useful
  decisions per wall second by `47%`. Generated primitives refined the best
  candidate from tick `136` to `135`, so this was not literal tape playback. A
  follow-up exposed the replay at four-tick rather than 16-tick resolution,
  preserving short setup inputs instead of averaging them into coarse action
  summaries. At only 16 learned decisions per seed it found `2/4` terminals
  and an authenticated tick-`130` candidate. At the full matched budget it
  found `4/4` terminals and retained tick `130`, versus `3/4` and tick `135`
  for the coarse replay, while visited states increased from `555` to `745`.
  Ordinary suboptimal replay therefore improves terminal sample efficiency and
  does not cap the learned policy: generated actions beat the `131` sample.
  A subsequent generic cold-start correction stopped treating pre-terminal
  sparse action cost as goal evidence and instead acquired
  least-expanded, semantically diverse frontiers. At the full matched scratch
  budget it increased useful decisions from `128` to `153`, useful training
  transitions from `513` to `572`, and visited states from `571` to `668`,
  but still found `0/4` terminals and stopped at the same roughly
  `1,496`-distance frontier. Cold-start acquisition was a real throughput
  defect, but is not the terminal barrier. Extending ordinary relative-heading
  primitives from a fixed four ticks to the same `4/8/16/40` duration lattice
  increased diagnostic useful decisions from `32` to `47` and useful training
  transitions from `115` to `157`, but again found `0/4` terminals and
  regressed the best proposal frontier from roughly `1,496` to `1,705`.
  A 64-decision, eight-proposal single-seed saturation probe then evaluated
  `511` native transitions, including `54` relative-heading proposals spanning
  every `4/8/16/40`-tick duration, and still stopped at the same roughly
  `1,496` frontier. Primitive-duration availability and simple sampling width
  are not the terminal barrier. Evidence:
  `ordon-p3-q131-local-replay-ablation-v1.report.json`.
- A distinct goal-relabeled fitted-Q treatment now turns exact replay-achieved
  endpoints into temporary goals and learns only negative native ticks along
  exact predecessor edges. Relabeled rows have zero native-terminal and
  promotion authority. On a matched 4x16 scratch diagnostic it improved the
  current local control's best proposal depth from roughly `1,790` to `1,611`
  and visited `278` states instead of `248`, but produced fewer useful training
  transitions (`122` versus `152`) and no terminal in either cell. At the full
  4x64 budget it again found `0/4` terminals and converged on the same roughly
  `1,496` frontier, while spending `143` seconds in model updates. Generic
  hindsight reachability is now real and ablatable, but this formulation does
  not solve the terminal barrier. Evidence:
  `ordon-p3-q131-local-replay-ablation-v1.report.json`.
- Supported achieved-goal return now outranks cold-start novelty when choosing
  among equally fresh retained frontiers, without granting native-terminal or
  promotion authority. In the matched 4x64 native follow-up, all `49`
  retained-frontier acquisitions had achieved-goal support. It found no
  terminal, but broke the recorded roughly `1,496`-distance plateau with a
  `1,322.9` proposal from the replay's westward envelope, increased useful
  decisions from `140` to `149`, and increased useful training transitions
  from `538` to `545`. That best route consumed the complete `160`-tick
  discovery horizon. The next diagnostic is therefore a larger discovery
  horizon with native terminal, promotion, and final tick authority unchanged,
  not more mining at the truncated horizon. Evidence:
  `ordon-p3-q131-local-replay-ablation-v1.report.json`.
- The `160`-tick discovery horizon was only `5.3` seconds at 30 Hz and
  invalidated the inference that scratch could not reach the terminal. A
  sealed `256`-tick scratch probe found and retained a real load-zone hit at
  tick `206` on decision `15`. By decision `20`, learned terminal value caused
  the policy to greedily select a terminal-producing action at Q `98.824`;
  `10` later retained-frontier acquisitions had native terminal support. No
  demonstration transitions or authored route coordinates were used. Scratch
  terminal learning is therefore real and replay-independent, but the route is
  still slow. Discovery horizons must be generous or adaptive; the
  `131`/`125` thresholds govern shortening and promotion, not whether initial
  exploration may continue. Evidence:
  `ordon-p3-q131-local-replay-ablation-v1.report.json`.
- Unassisted learned tactic routing now requires an explicit
  `from_scratch_discovery` request and a route horizon of at least 30 native
  seconds (`900` ticks) and at least twice the promotion target. The unattended
  native goal-learning loop uses the same minimum. A local-refinement request
  can no longer silently stand in for adequate scratch discovery.
- Four isolated learners then ran under a sealed `from_scratch_discovery`
  request with a `1,024`-tick (`34.1`-second) horizon, no incumbent tape, no
  demonstration, no authored route coordinates, and no cross-seed replay.
  Two independently reached the authenticated load zone: seed `155921`
  reproduced tick `206`, and seed `130363` found tick `337` through a selected
  `generalized_value` action. The `2/4` terminal rate proves reproducible
  scratch learning but is not yet reliable, and neither route clears the
  tick-`131` promotion threshold. Evidence:
  `ordon-p3-q131-local-replay-ablation-v1.report.json`.
- Doubling the isolated decision budget to `128` left terminal discovery at
  `2/4`. Tick `206` did not improve and tick `337` improved only to `336`;
  failed seeds moved closer but still did not terminate. All `37`
  terminal-supported frontier acquisitions had no predicted total terminal
  ticks because the zero-terminal achieved-goal critic incorrectly remained
  active after real terminal support existed. Discovery and terminal
  optimization now use distinct critics. The exact tick-`206` seed reproduced
  its terminal, and all `9` later terminal-supported frontier acquisitions
  carried learned total-tick predictions instead of nulls. The best route
  remained tick `206`, and predictions ranged from `51` to `211`, so terminal
  credit is now operational but not yet calibrated or route-improving.
- Shared replay had coupled training evidence to exploration coverage: every
  peer transition entered every lane's frontier archive and visited-state set.
  Under the full `4x64` plan this correlated all lanes onto the same failed
  corridor and found `0/4` terminals. Training replay and immutable critics now
  remain global while frontier routes and visitation coverage remain
  lane-local. Under the identical plan, the corrected treatment found
  authenticated terminals in `3/4` seeds at ticks `228`, `332`, and `246`;
  all `18` terminal-supported acquisitions carried learned total-tick
  predictions. Reliable scratch discovery is now established without
  demonstration or incumbent tape, but the best scratch route is still
  `97` ticks slower than the `131` replay. Evidence:
  `ordon-p3-q131-local-replay-ablation-v1.report.json`.
- The sealed tactic feature schema now exposes current velocity and speed,
  recent straightness and momentum retention, contact plus measured slowdown,
  camera orientation, exact A-button prompt-status bits, and the native
  front-roll acceptance prompt (`BUTTON_STATUS_UNK_121`, `0x79`). Executable
  action features independently encode heading, duration, camera/target
  modifiers, button timing, and roll cadence. Prompt status is learner state,
  not an executor gate: simultaneous and staggered inputs remain explorable.
  These are observations and diagnostics only; learned delayed return remains
  the sole utility ordering.
- Replay mining, entry-conditioned validation, promoted-tactic storage, and
  primitive-versus-macro competition exist. No tactic has yet earned promotion
  because the held-out probe contained no successful exploration observation.
  P4 therefore lacks demonstrated search value. Evidence:
  `ordon-p4-entry-conditioned-promotion-v1.report.json`.

A terminal hit, reliable terminal hit, `125` tie, or faster search for the same
tie is diagnostic evidence only.

## Non-negotiable boundaries

- Production modules have one reason to change. Organize related behavior in
  named module folders; do not use grab-bag `utils`, `common`, or `misc`
  modules.
- A production Rust source file must stay below 1,500 physical lines, with a
  normal target below 1,000. Existing oversized files are shrinking debt, not
  precedent.
- Prefer typed collaborators with narrow ownership over sprawling functions.
- JSON is not an operational checkpoint, replay, transition, model, frontier,
  journal, or learned-tactic format. Small authored requests and exported
  reports may use JSON.
- Every evaluated native transition is eligible learning evidence. Every exact
  terminal candidate is retained independently of policy selection.
- Evaluation results never retroactively replace the policy-selected action.
- Primitive actions remain generic and state-local: analog direction,
  duration, camera modifier, and currently available prompted buttons.
- Learned tactics compete with primitives; promotion never removes primitive
  exploration.
- Performance reports include process launch, simulation, restore, checkpoint
  capture, learner update, orchestration, persistence, and evidence projection.
- First-hit comparisons bind the same source checkpoint, terminal predicate,
  game bytes, card fixture, fidelity, and source boundary.

## P3 - Prove delayed-credit continuous-control learning

- [ ] Use native-terminal return and retained intermediate states to shorten
      the authenticated tick-`206` scratch route below the tick-`131`
      demonstration under separate promotion authority.

Exit gate:

- Learned return is the only action-utility ordering.
- The learned policy reliably escapes the around-corner local optimum.
- From-scratch learning reaches the real load zone, and demonstration-assisted
  learning produces an authenticated route strictly better than the `131`-tick
  degraded demonstration.

## P4 - Demonstrate tactic discovery and reuse

- [ ] Collect independent successful replay support sufficient to test
      promotion; do not weaken terminal-only promotion merely to manufacture a
      promoted tactic.
- [ ] Prove that mining can discover and parameterize useful compositions from
      primitives. A key generic case is direction selection plus a one-frame
      camera/target modifier followed by sustained forward input; it must not
      be hard-coded as an Ordon macro. Roll cadence and curved/waypoint steering
      must use the same composition mechanism rather than bespoke rewards.
- [ ] Promote only when a composition improves terminal/tick return on held-out
      source-state groups relative to executing its primitive components.
- [ ] Verify the promoted tactic's typed entry conditions, bounded execution,
      emitted controller input, outcome distribution, and exact replay lineage.
- [ ] Run matched promotion-enabled and primitives-only campaigns and measure
      useful transitions per wall second, terminal discovery rate, and
      time-to-best-route on held-out seeds.

Exit gate:

- At least one useful composition is discovered without hard-coding an Ordon
  route, reproduces exactly, and provides measurable held-out search value.
- Primitive actions remain selectable everywhere they are valid.

## P5 - Establish the capacity curve

- [ ] After the P2 hot-path change and P3 learner selection, run one sealed
      fixed plan at 1, 2, 4, 8, and 16 workers.
- [ ] Report proposal parallelism, environment parallelism, and learner-update
      work separately.
- [ ] Plot useful transitions, learner updates, unique frontier cells, native
      ticks, checkpoint traffic, best first-hit tick, wall time, and peak
      memory.
- [ ] Identify the saturation point and responsible resource: simulation,
      checkpoint capture/transfer, replay, learner fit, persistence, scheduling,
      or duplicated exploration.
- [ ] Demonstrate a 10x improvement in time-to-fixed-useful-evidence against the
      recorded baseline, or publish the measured limiting curve that prevents
      it.

Exit gate:

- Worker growth improves useful evidence rather than merely executed ticks.
- No 100x capacity plan is proposed until the 10x curve is understood.

## P6 - Beat and verify the authenticated Ordon incumbent

- [ ] Produce a machine-generated candidate whose first authenticated load-zone
      hit is strictly earlier than tick `125` from boundary `506`.
- [ ] Continue to a credibility target of tick `123` or lower. Tick `124`
      clears the formal regression gate but is weak evidence for harder routes.
- [ ] Minimize the complete successful controller tape without changing source,
      terminal, game build, fixture, or execution fidelity.
- [ ] Cold-replay the complete minimized tape at least twice from process boot
      with learner and tactic executors out of the loop.
- [ ] Require byte-identical input and identical authenticated terminal evidence
      across cold replays.
- [ ] Publish execution plan, learner/replay lineage, complete timing, worker
      topology, peak memory, winning lineage, first-hit tick, and proof
      identities.

Anything short of the sub-125 cold-replay proof is progress evidence, not task
completion.
