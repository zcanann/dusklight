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
- Checkpoint retention is the measured throughput bottleneck. Each ordinary
  retained machine image is about 295 MB and takes roughly 4.4-5.5 seconds for
  the full retention operation, while the host checkpoint handle itself is
  only 224 bytes. Exact direct-restore/replay parity is already proved.
  Evidence: `ordon-p2-checkpoint-path-v2.report.json`.
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
- The current generalized value model is not the leading held-out control.
  A continuous fitted-Q forest performed better on unseen action realizations;
  neural Double-Q and conservative Q extrapolated poorly. This is evidence for
  the next learner experiment, not permission to promote a model without
  matched native validation. Evidence:
  `ordon-p3-control-comparison-v1.report.json`.
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

## P2 - Remove checkpoint retention from the hot path

- [ ] Redesign ordinary selected-endpoint retention so a persistent worker can
      continue from its process-local checkpoint without synchronously copying
      and persisting a full portable machine image at every decision.
- [ ] Preserve bounded per-worker residency, explicit byte accounting,
      eviction reasons, process-loss handling, and exact authenticated replay
      fallback. Do not replace the current bounded cache with hidden emulator
      copies.
- [ ] Make portable recovery material lazy or cadence-bound while retaining
      exact interruption recovery from the authenticated root and input
      lineage.
- [ ] Repeat the representative early/middle/late benchmark and a fixed route
      campaign with capture, transfer, persistence, restore, eviction, replay,
      native simulation, and wall time reported separately.

Exit gate:

- Ordinary warm follow-ups use direct restore without a full-image capture on
  the decision hot path.
- Direct continuation and replay fallback produce byte-identical decoded
  transitions, terminal evidence, and authenticated terminal boundaries.
- Orchestration plus persistence no longer dominates native simulation on the
  fixed throughput campaign.

## P3 - Prove delayed-credit continuous-control learning

- [ ] Put the best held-out candidate learner into the live policy path behind
      a sealed treatment identity. Start with continuous fitted Q because it
      currently beats the local generalized model; retain the existing model
      and structured search as controls.
- [ ] Feed the learner the complete typed observation and action-availability
      state needed to infer momentum, straight travel, productive versus
      slowing contact, rolling opportunities, prompted actions, and camera
      state. Add missing signals as observations or auxiliary targets, never as
      shaped reward.
- [ ] Run matched from-scratch, ordinary-demonstration, and structured
      non-learning campaigns with equal native budgets and identical source,
      terminal, and execution bindings.
- [ ] Measure held-out value error, uncertainty, action ordering, useful
      transitions per wall second, terminal discovery rate, and best
      authenticated tick across independent seeds and semantic state groups.
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
