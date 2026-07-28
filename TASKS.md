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
  candidate from tick `136` to `135`, so this was not literal tape playback.
  P3 still fails: the candidate did not beat `131`, scratch still required
  replay support to succeed at this capacity, and the newer generic
  executable-factor coverage also found no scratch terminal in its diagnostic
  cell. A subsequent generic cold-start correction stopped treating
  pre-terminal sparse action cost as goal evidence and instead acquired
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

- [ ] Run a matched scratch ablation of the goal-relabeled fitted-Q treatment.
      It must learn only negative native ticks to exact replay-achieved goals,
      generalize that return to the authored goal-conditioned state, and retain
      zero terminal/promotion authority until native evaluation reaches the
      real terminal.
- [ ] Demonstrate that ordinary suboptimal human replay improves sample
      efficiency without capping the policy or becoming required for success.

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
