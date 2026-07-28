# Active tasks: make route learning auditable, scalable, and effective

This file contains only unfinished learning-framework work. Completed
implementation belongs in Git; benchmark history belongs in immutable reports.

## Product objective

Build a generic checkpointable optimizer that:

1. starts from an authenticated native checkpoint;
2. observes typed game state, action availability, and measured trajectories;
3. proposes primitive controller actions and learned tactic compositions;
4. evaluates alternatives from retained native states;
5. shares authenticated transitions with a state/action learner;
6. discovers and validates reusable tactics; and
7. minimizes native input ticks to an authenticated terminal.

The UI is not a TAS authoring surface. A human recording may be optional
experience, but it must not define privileged actions, observations, rewards,
state, or terminal semantics.

## Current benchmark truth

Ordon is the acceptance benchmark. The eligible request starts at authenticated
boundary `506`; the human incumbent first reaches the actual load-zone terminal
at tick `125`.

- The best machine result currently ties first-hit tick `125`. It does not beat
  the benchmark.
- Four lanes with four proposals each completed 128 decisions and 10,203
  evaluated native ticks in 202 seconds, yielding 373 useful transitions.
- One lane widened to sixteen proposals took 339 seconds for only 32 decisions.
  Wider sibling batches are not a substitute for learner iterations.
- The native checkpoint primitive is functional and authenticated. A dedicated
  macOS early/middle/late benchmark now separates process launch, root replay,
  direct restore, checkpoint capture, in-process host snapshot transfer, and
  fact extraction. Direct continuation took 0.87-0.91 seconds under full
  per-tick state-hash proof, versus 7.8-50.4 seconds for portable root replay.
- The fixed 1/2/4-worker online-replay comparison now passes. With one
  campaign-owned fitter and immutable snapshots, learner-update throughput rose
  from 40,745 to 58,933 to 76,515 per-second-millionths, while useful-transition
  throughput rose from 165,377 to 239,198 to 328,564. Every cell published 18
  snapshots, performed 17 centralized updates, and performed zero lane-local
  fits.
- Parallel held-out macro validation removed the serial post-campaign tail:
  validation wall time fell from 192.7 seconds at one worker to 96.2 seconds at
  two and 49.1 seconds at four. Full measured wall time fell from 417.2 to
  288.5 to 222.2 seconds.
- Multi-seed lanes now retain the policy-selected endpoint and route follow-up
  work to its owning persistent worker. A two-worker/four-seed macOS stress run
  traced every lane as authenticated root, process-local checkpoint,
  process-local checkpoint. Six of eight follow-ups encountered explicitly
  reported bounded-cache eviction and reconstructed the exact source by
  authenticated replay instead of aborting.
- Checkpoint capture is now the measured P2 bottleneck. The 12-decision stress
  run spent 54.2 seconds capturing 18 campaign checkpoints; its two-entry,
  640-MiB-per-worker caches evicted 14 entries during campaign work.
- The v2 representative checkpoint benchmark confirms the same cause: each
  294,694,748-byte machine image took 1.77 seconds to capture and 4.40-5.53
  seconds for the whole retention operation. The 224-byte host snapshot took
  only 94-98 microseconds to capture and move into the resident cache. Exact
  decoded learning transitions, terminal-evidence bytes, and terminal boundary
  fingerprints matched direct restore and authenticated replay at route ticks
  15, 62, and 109.
- The checkpoint-wide semantic digest now covers all 21 registered
  parity-relevant entries while canonicalizing identified presentation-only
  state: JUT/VI clocks, JUTProcBar, the particle presentation heap, and explicit
  host-ABI padding. Direct and portable continuation digests match at all three
  representative frontiers with no divergent entries; raw checkpoint capture
  and restore remain byte-exact. Evidence:
  `ordon-p2-checkpoint-path-v2.report.json`.
- The macOS headless audit now proves the execution boundary instead of
  inferring it from launch flags. Host pacing and the host audio device are
  disabled, while ImGui frame lifecycle and CPU GX submission are suppressed
  on candidate ticks; GPU frames were already discarded before encoding.
  Gameplay draw traversal, deterministic DSP/JAS emulation, and game sound
  updates remain authoritative. The v3 early/middle/late benchmark measured
  zero CPU renderer-submission time and exact 21-entry semantic, transition,
  terminal-evidence, and boundary parity at route ticks 15, 62, and 109.
  Representative root replays spent 4.9-29.7 ms in draw traversal, 2.9-25.7 ms
  in audio emulation, and 0.1-1.1 ms in game audio updates. Total proof-mode
  wall time remained effectively unchanged from v2 (1,143.3 versus 1,142.3
  seconds), confirming that state proof and checkpoint work dominate rather
  than presentation. Evidence: `ordon-p2-headless-path-v1.report.json`.
- A fresh current-code arm64 macOS campaign now feeds a deterministic
  group-isolated value audit. Complete spatial regions and complete typed
  action realizations are assigned to disjoint train, conformal-calibration,
  and test partitions; serialized overlap and coverage claims are recomputed
  on load. On the initial 18-transition corpus, spatial test coverage was
  100% at the 90% target, but unseen-action coverage was only 80% and the
  single comparable unseen-action ranking lost with 0.083 observed regret.
  This is a P3 failure signal, not acceptance. Evidence:
  `ordon-p3-generalized-calibration-v1.report.json`.
- A larger 36-transition seed now cross-calibrates every complete semantic
  group once as test and once as validation. The robust conformal scale is the
  maximum of the pooled quantile and each whole-fold quantile, so one
  distribution-shifted fold cannot disappear inside the aggregate. Spatial
  and unseen-action test coverage both reached 100% against the declared 90%
  target, at mean interval radii 0.321 and 0.635. This calibrates uncertainty
  but does not excuse poor unseen-action ordering: only 3 of 12 comparable
  rankings won. Evidence: `ordon-p3-cross-calibration-v1.report.json`.
- The same disjoint partitions now compare the live local generalized model
  with a continuous fitted-Q forest, neural Double-Q, conservative offline Q,
  and a structured non-learning control. On unseen action realizations the
  forest reduced MAE from 0.188 to 0.137 and won the only comparable ranking;
  the live model lost it. Double-Q and conservative Q extrapolated
  catastrophically across the held-out spatial region and exposed correspondingly
  large critic disagreement. Evidence:
  `ordon-p3-control-comparison-v1.report.json`.
- A matched current-code macOS search diagnostic compared learned
  from-scratch, structured non-learning, and learned demonstration-assisted
  cells at the same seed, width, decisions, and native binding. Structured
  search beat learned scratch on useful decisions (4 versus 3) and useful
  transitions (9 versus 4). Demonstration replay raised those to 6 and 24 but
  cost 311 versus 207 seconds; no cell reached the terminal, so neither
  learned superiority nor improvement beyond demonstration is established.
  Evidence: `ordon-p3-matched-controls-v1.report.json`.
- A deeper matched ablation against the explicitly degraded 131-tick
  demonstration reached 16 decisions and 64 proposals per cell. Ordinary demo
  replay increased useful decisions from 10 to 16, useful transitions from 41
  to 68, and visited states from 11 to 21; scratch found no terminal, while
  demo-assisted search independently retained one exact terminal. That
  candidate cost 137 ticks, however, so the demo helped discovery but did not
  establish improvement beyond itself. Evidence:
  `ordon-p3-q131-ablation-v1.report.json`.
- A current-code audit found that those q131 cells used a generation barrier:
  every decision consumed the same revision-zero snapshot, no decision exposed
  finite Q, and fitting happened only after the campaign. Learned routes now
  consume each newly fitted immutable snapshot at the declared refit cadence.
  In corrected 16-decision macOS cells, scratch exposed finite Q on 15
  decisions across 9 snapshots and demonstration-assisted search exposed it on
  14 decisions across 8 snapshots. Neither reached the terminal. A follow-up
  four-decision diagnostic bound nonzero cyclic demonstration frontiers to
  terminal-support acquisition and restored duration-diverse `4/16/4/24`
  proposals, improving the best near-terminal sibling from goal distance
  513.69 to 151.34, but still found no terminal. This validates live learning
  mechanics while leaving delayed-credit acceptance failed. Evidence:
  `ordon-p3-live-replay-ablation-v1.report.json`.
- Terminal-support acquisition now reserves the highest learned candidate for
  each currently available prompted-button mask before filling remaining
  duration/type slots. On the same four-decision macOS diagnostic, the critical
  batch changed from `none/L+A/A/none` to `none/A/L+A/L`; the target-only action
  reached the terminal in 25 ticks and was independently persisted even though
  forced exploration selected another action. The complete suffix still cost
  137 frames versus the 131-frame demonstration, so prompted-factor discovery
  and retention now work while learner selection and improvement remain open.
  Evidence: `ordon-p3-prompted-factor-coverage-v1.report.json`.
- A matched 16-decision arm64 macOS audit then exposed two policy wrappers
  overriding that learned success. `DemonstrationFrontierOnce` forced epsilon
  again when revisiting an already-expanded frontier, and generalized
  acquisition subsequently displaced the supported exact greedy action.
  Interventions are now restricted to expansion zero, and exact greedy or
  epsilon authority is preserved over interpolated acquisition. On the fixed
  cell's repeated frontier, the exact action had `best_q = selected_q = 99.75`,
  was selected greedy, and authoritatively reached the terminal. The suffix
  still cost 137 frames versus the 131-frame demonstration, so basic
  learn-and-exploit behavior is demonstrated while route improvement remains
  failed. Evidence: `ordon-p3-greedy-authority-v1.report.json`.
- The first two-seed replay-macro probe mined 26 candidates and completed 52
  held-out macro-versus-primitive comparisons. It spent 1,008 validation ticks
  and 149.4 seconds—about 26% of total wall time—but promoted nothing and
  emitted no reusable action. Macro discovery is therefore implemented
  machinery, not yet demonstrated search value. Evidence:
  `ordon-p4-macro-promotion-probe-v1.report.json`.
- A matched current-code arm64 macOS rerun now mines only authenticated option
  prefixes and connected decision sequences, excludes exact source states, and
  admits held-out frontiers through typed entry semantics. Candidates fell from
  26 to 12, comparisons from 52 to 17, validation ticks from 1,008 to 508, and
  validation wall time from 149.4 to 51.1 seconds. No exploration observation
  reached the terminal, so terminal-only promotion correctly produced no
  reusable action. The machinery is cheaper and semantically defensible, but
  P4 search value remains unproved. Evidence:
  `ordon-p4-entry-conditioned-promotion-v1.report.json`.

A terminal hit, a reliable terminal hit, a 125 tie, or a faster search for the
same tie is diagnostic evidence only.

## Non-negotiable boundaries

- Production modules have one reason to change. Organize related behavior in
  named module folders; do not use grab-bag `utils`, `common`, or `misc`
  modules.
- A production Rust source file must stay below 1,500 physical lines, with a
  normal target below 1,000. Test modules belong in adjacent test files once
  they materially obscure production behavior. Existing oversized files are
  debt to split, not precedent for raising the limit or adding more code.
- CI must reject new oversized files and any growth in a grandfathered
  oversized file. Every cleanup milestone lowers the grandfathered ceiling
  until no exception remains.
- Prefer typed collaborators with narrow ownership over functions that accept
  sprawling configuration/state argument lists. A module boundary must reflect
  responsibility and data ownership, not merely move lines behind `include!`.
- Reward is authenticated terminal value minus native input cost. Trajectory,
  velocity, collision, straightness, rolling, and prompted-action availability
  are observations or auxiliary prediction targets, not handcrafted utility.
- JSON is not an operational checkpoint, replay, transition, model, frontier,
  journal, or learned-tactic format. Small authored requests and exported
  reports may use JSON.
- Every evaluated native transition is eligible learning evidence. Every exact
  terminal candidate is retained independently of whether the policy selected
  it. Evaluation results must never retroactively replace the policy action.
- Human experience is ordinary, ablatable replay. The system must retain a
  from-scratch lane and must be able to improve beyond the demonstration.
- Performance reports include process launch, native simulation, restore,
  checkpoint capture, learner update, orchestration, persistence, and evidence
  projection. Do not move overhead outside the measured boundary.
- First-hit comparisons must bind the same source checkpoint, terminal
  predicate, game bytes, card fixture, fidelity, and source boundary.

## P2 - Make native checkpointing buy throughput

- [x] Audit headless execution to prove which renderer, audio, pacing, and
      presentation systems still run. Disable only work whose removal preserves
      native state and terminal parity, then measure the result.
- [x] Narrow the checkpoint-wide semantic digest to parity-relevant native
      state, excluding automation allocator history, and prove that the direct
      and portable continuation digests then match.

Acceptance:

- After warm-up, ordinary non-root expansions use direct restore unless an
  explicitly reported ownership, eviction, or process-loss condition prevents
  it. Met by the owner-routed stress report and the representative checkpoint
  benchmark.
- The same transition and terminal evidence are byte-identical through direct
  restore and authenticated replay fallback. Met at early, middle, and late
  frontiers.
- Orchestration plus persistence no longer dominates native simulation on the
  fixed throughput benchmark.

## P3 - Prove that the learner solves delayed continuous-control credit

- [x] Compare the current local generalized model against at least:
  - fitted Q over a learned continuous representation;
  - a double-Q or ensemble control;
  - a conservative offline control; and
  - a non-learning structured-search baseline.
- [x] Calibrate value and uncertainty on held-out state regions and held-out
      action realizations, not random rows from the same correlated route.
- [x] Run matched demonstration-assisted and from-scratch ablations. The
      demonstration may improve sample efficiency but may not cap the policy at
      the demonstrated route.

Acceptance:

- Learned return is the only action-utility ordering.
- Auxiliary signals improve representation or prediction without becoming
  reward shaping.
- The learner reliably escapes the around-corner local optimum and improves a
  suboptimal demonstrated route.

## P4 - Discover and reuse tactics instead of blessing a fixed list

- [ ] Keep primitive actions generic and state-local: analog direction,
      duration, camera modifier, and currently available prompted buttons.
- [ ] Mine repeated successful action subsequences and parameter relationships
      from authenticated replay.
- [ ] Promote a tactic only after it improves terminal/tick return on held-out
      source states relative to its primitive components.
- [ ] Represent promoted tactics with typed entry conditions, bounded execution,
      emitted controller input, outcome distributions, and exact lineage.
- [ ] Allow primitive and promoted tactics to compete under the same learner;
      promotion must not permanently remove primitive exploration.
- [ ] Measure whether promotion improves useful transitions per wall second and
      time-to-best-route on held-out seeds.

Acceptance:

- Useful compositions can be discovered without hard-coding an Ordon route.
- A promoted tactic reproduces exactly and provides measurable held-out search
  value.

## P5 - Establish the capacity curve before buying 10x or 100x volume

- [ ] Run fixed-plan scaling trials at 1, 2, 4, 8, and 16 workers after P0-P2.
- [ ] Separate proposal parallelism, environment parallelism, and learner-update
      parallelism in the report.
- [ ] Plot useful transitions, learner updates, unique frontier cells, native
      ticks, restore traffic, and best first-hit tick against wall time and
      memory.
- [ ] Identify the saturation point and the responsible resource: native
      simulation, restore bandwidth, learner fit, persistence, scheduling, or
      duplicated exploration.
- [ ] Demonstrate at least a 10x improvement in time-to-fixed-evidence or explain
      with measured scaling limits why additional hardware cannot provide it.
- [ ] Do not plan 100x capacity until the 10x curve shows useful near-linear
      scaling.

## P6 - Beat and verify the authenticated Ordon incumbent

- [ ] Produce a machine-generated candidate whose first authenticated load-zone
      hit is strictly earlier than tick `125` from boundary `506`.
- [ ] Continue to a credibility target of tick `123` or lower; a tick-124 result
      clears the formal regression gate but is not strong evidence that the
      framework is ready for harder routes.
- [ ] Minimize the complete successful controller tape without changing source,
      terminal, game build, fixture, or execution fidelity.
- [ ] Cold-replay the complete minimized tape at least twice from process boot
      with the learner and tactic executors out of the loop.
- [ ] Require byte-identical input and identical authenticated terminal evidence
      across cold replays.
- [ ] Publish the execution plan, learner/replay lineage, complete timing,
      worker topology, peak memory, winning route lineage, first-hit tick, and
      proof identities.

Anything short of the sub-125 cold-replay proof is progress evidence, not task
completion.
