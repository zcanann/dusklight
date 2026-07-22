# Active task: ship a usable deterministic TAS optimization loop

## Outcome

Given an existing TAS segment, its source boundary, an authored terminal
predicate, and an optional incumbent tape, one unattended campaign must be able
to:

1. restore the exact source state without replaying unrelated history;
2. generate precise raw-PAD candidates, including coordinated continuous edits;
3. execute them through persistent native workers without rendering or host
   pacing affecting simulation;
4. retain complete success and failure trajectories as learning experience;
5. improve its proposals across generations;
6. expose live candidates as ordinary ephemeral siblings in the Route Workbench;
7. promote a winner only after exact cold replay from boot.

The first benchmark is the checked 125-tick `ToOrdonSprings` segment. It is a
small control problem intended to expose framework defects cheaply. It is not a
claim that Ordon-specific machinery is useful elsewhere.

## Current truth

Working foundations:

- Absolute TAS playback is deterministic under the current fixture and timing
  model. Any repeat divergence remains a framework bug.
- Intermediate checkpoints have validated restore/replay windows and persistent
  workers can reuse them across batches.
- Eight workers have measured approximately 1,027 transitions/second and 8.22
  125-tick episodes/second on the development host with no GPU submissions.
- Native batches retain phase-correct pre-input observations, exact chosen and
  consumed PAD, post-simulation state, terminal evidence, and content-bound
  episode shards.
- Replay generations, lineage, held-out splits, complete actor populations,
  collision/geometry views, and several trainable representation baselines exist.
- A lossless factorized PAD schema and matching Rust/C++ decoder cover precise
  sticks, analog channels, buttons, edges, and duration.
- A bounded frozen dense-model format and allocation-free native inference path
  are compile- and contract-tested.
- A real state-reactive frozen policy now completes live native execution,
  byte-exact Rust reinference, ordinary-tape realization, and model-free cold
  replay with identical full-channel gameplay state across all 508 boundaries.
- A sealed optimization-request contract now binds route lineage, incumbent,
  source boundary, terminal predicate, independent exploration/promotion
  horizons, budgets, execution seeds, proposal schemas, optimizer, resume
  location, and retention policy. The first Ordon q125 CEM request validates
  against the checked-in route and uses 160 ticks for exploration while keeping
  promotion strictly sub-125.
- Completed native campaign evaluations now retain the exact terminal engine
  boundary and project a bounded set of authenticated successes and recent
  misses as ephemeral Workbench siblings. Those candidates use ordinary
  playback and thumbnail capture, and disappear on refresh when their campaign
  artifacts are removed.
- Optimization resume state now has a synced append-only hash-chain journal for
  sealed candidates, completed evaluations, and optimizer checkpoints. It
  recovers a torn final record, rejects complete-record tampering and duplicate
  compiled tapes, preserves uncheckpointed results for deterministic replay
  into the last optimizer snapshot, and validates ordered event batches in
  memory before one durable append/refold so large generations do not require
  one full journal pass per candidate.
- The residual campaign command now accepts a sealed native execution binding,
  owns one persistent checkpoint worker per deterministic lane for the complete
  run, dispatches exact horizon-extended residual tapes as native suffix
  batches, adopts completed crash-window artifacts, and checkpoints native
  evidence plus optimizer/archive state without relaunching per generation.
- The Route Workbench now projects checked optimization requests on their owning
  segments and exposes an explicit typed run/resume action. Rust validates and
  materializes immutable native execution authority, runs the persistent
  campaign off the HTTP thread, and reports ready, running, resumable,
  completed, invalid, or failed state without launching a hidden default job.
- Campaign cards read bounded authenticated resume/checkpoint summaries rather
  than rescanning candidate directories. They expose queued and completed work,
  generations, charged ticks, exact successes, current best, retained failures,
  proposal source, workers, uncheckpointed completions, and explicit runtime or
  artifact blockers while remaining cheap to poll.

Not yet working:

- There is no closed `collect -> train -> freeze -> execute -> ingest -> refit`
  campaign.
- The 125-tick Ordon incumbent remains unbeaten.
- The historical 18,867-candidate Ordon campaign was local procedural mutation,
  not evidence of successful learning.
- The current learned proposer is an offline window-patching system over a coarse
  action catalog, not a continuous state-reactive policy.

Known reasons the earlier Ordon search produced no improvement:

- Candidate exploration stopped at the 125-tick promotion threshold, turning
  slower but successful perturbations into indistinguishable failures.
- Goal feasibility is a ranking cliff. Failed candidates receive too little
  information to preserve coordinated repairs across generations.
- Most candidates changed one compressed PAD run or one window. A better corner
  can require an approach change followed by downstream camera, facing, and roll
  repairs.
- Learned actions were quantized to sixteen full-magnitude headings and button
  banks, discarding the fine analog control required for route golf.
- The inspected Q corpus was too small and sparse to support counterfactual
  action values.
- The six-lane learned-proposal allocator defect is fixed; proposal quality and
  coordinated downstream repair remain the measured limitations.

Primary evidence remains in:

- `docs/glitch-hunting/throughput.md`
- `docs/glitch-hunting/benchmarks/intro-route.md`
- `docs/glitch-hunting/benchmarks/intermediate-checkpoint-validation-20260721.json`
- `docs/glitch-hunting/benchmarks/factorized-policy-online-20260721.json`
- `docs/glitch-hunting/benchmarks/frozen-policy-gate1-20260721.json`
- `docs/glitch-hunting/benchmarks/macos-worker-scaling-20260721.json`

Historical implementation detail remains available in Git. This file records
current work and acceptance gates, not every completed experiment.

## Invariants

- Identical initial state and consumed PAD must produce identical per-tick
  gameplay state. Mine no workaround for a repeatability failure.
- Normal automation may read game state and supply controller input. It must not
  patch gameplay state. Checkpoints accelerate experiments but do not authorize
  a result.
- Every promoted result is the exact realized PAD sequence and must replay from
  ordinary cold boot without a policy, checkpoint, or gameplay write.
- Authored terminal predicates and simulated ticks are authoritative. Rewards,
  values, geometry, novelty, and demonstrations may guide proposals but cannot
  declare success.
- Exploration horizon and promotion threshold are different values. A campaign
  may learn from slower successes while promoting only a faster result.
- Raw PAD is the ground-truth action. Model outputs, tactics, curves, and residual
  parameters must compile to and record every consumed frame.
- Rendering, shader compilation, host pacing, and orchestration time never enter
  the TAS score.
- Compare methods under the same simulated-tick budget and initial-state
  distribution. Algorithm names and training loss are not results.
- Missing observation fields, unsupported actions, disabled learning, shortened
  proposal batches, and fallback proposal sources must be visible in artifacts
  and status output.
- Add observations, tactics, model capacity, or workers only in response to a
  measured bottleneck or a generic capability requirement.

## 1. Restore a green native execution boundary

- [x] Build the game and native automation runner against the repository-recorded
      Aurora revision. A fresh macOS Debug build of `dusklight` and the suffix,
      factorized-PAD, frozen-inference, and native-policy-feature test targets
      succeeds against clean public Aurora commit
      `ce4baccedb2aabddce5b552f0573674e857fb7c3`; the Rust workspace builds too.
      No compile or API mismatch remains, and the stale local-only/dirty-submodule
      claim has been removed.
- [x] Fix the six-lane proposal-budget remainder calculation and add property
      tests proving every requested budget is allocated exactly for small and
      large population sizes. The allocator now derives its remainder from the
      six-lane array itself; exhaustive totals through 512 and representative
      populations through 10,000,003 prove exact, balanced allocation under
      every lane rotation.
- [x] Run one deliberately simple frozen policy through suffix-batch v5 from a
      validated checkpoint.
  - The native runner must capture the exact pre-input feature row.
  - Native inference must emit and consume one factorized PAD frame per tick.
  - The episode shard must retain model, feature, action, objective, checkpoint,
    chosen-PAD, and consumed-PAD identities.
- [x] Recompute every policy output in Rust from the captured feature rows and
      require byte-identical decoded PAD.
- [x] Cold-replay the realized tape without loading the model and require the
      same terminal verdict and complete gameplay-state sequence.
      The ordinary 508-frame realized tape was launched from a fresh process
      without a model or checkpoint. Its complete 13-channel pointer-free trace
      is byte-identical to the live policy trace across all 508 boundaries; all
      eight policy PADs and the full authored terminal predicate artifact match.
      The sealed verifier rejects tape, state, terminal, shard, reinference, or
      identity divergence; see `frozen-policy-gate1-20260721.json`.
- [x] Publish inference, head-decode, observation, simulation, restore, and
      episode-encoding costs from that live run.
- [x] Fail closed when the model, feature schema, action schema, objective,
      checkpoint, or captured feature width differs.

**Gate 1:** a real state-reactive native policy produces an ordinary tape whose
captured inference can be reproduced exactly in Rust and whose gameplay can be
reproduced exactly from cold boot.

## 2. Establish the continuous residual baseline

This baseline tests the harness and proposal surface before another learning
algorithm is credited or blamed. It is demonstration-seeded optimization, not
goal-only discovery.

### 2.1 Campaign contract

- [x] Define one content-bound optimization request containing:
  - timeline, lineage, segment, source boundary, and terminal predicate;
  - incumbent tape and incumbent first-hit tick when supplied;
  - exploration horizon independent of the promotion threshold;
  - candidate and simulated-tick budgets;
  - worker count, deterministic seeds, repetitions, and fidelity settings;
  - proposal/action schema and optimizer configuration;
  - resume location and artifact-retention policy.
- [x] Default the first Ordon campaign to an exploration horizon with enough
      slack to retain perturbed successes. Promotion remains strictly sub-125.
      The sealed `ordon-q125-residual-campaign.request.json` explores through
      tick 160 while retaining the incumbent first-hit tick and strict
      `promotion_before_tick = 125` as separate authenticated fields.
- [x] Make campaigns resumable after cancellation, worker crash, or UI closure
      without repeating sealed candidates or losing optimizer state.
      `native_residual_campaign_runner` now journals candidate batches before
      dispatch, revalidates and adopts complete result artifacts after a crash,
      allocates a fresh result path around partial artifacts, records every
      evaluation before the optimizer update, and restores the exact pending
      CEM generation. Focused tests cover deterministic residual-to-native
      conversion at frame 440 and non-overwriting crash-window recovery.

### 2.2 Residual action surface

- [x] Represent a candidate as the incumbent raw tape plus bounded additive
      residuals, then compile it losslessly to raw PAD before execution.
- [x] Cover main stick and camera stick direction and magnitude without heading
      quantization. Clamp only at the authentic PAD boundary.
- [x] Represent button presses as edge/hold edits and allow roll edges to shift,
      appear, or disappear without replacing the entire button schedule.
- [x] Supply deterministic temporal bases at several scales, initially exact
      frame, 2/4/8/16/32-frame windows, piecewise-linear ramps, and a small
      control-point curve.
- [x] Permit several simultaneous residuals so an early trajectory change can be
      accompanied by downstream repair in the same candidate.
- [x] Deduplicate on the compiled raw tape, not on optimizer parameters.
- [x] Record the exact intervention span and parent tape for attribution while
      keeping the realized tape authoritative.
      `dusklight-search::residual_action` now seals bounded candidates to the
      incumbent tape, composes analog and button edits deterministically, checks
      lossless raw-tape round trips, and deduplicates by realized tape digest.
      Compilation reports retain the parent/candidate/realized identities plus
      both the declared and exact realized intervention spans. The sealed Ordon
      request accepts only this implemented proposal schema and the implemented
      factorized raw-PAD action schema.

### 2.3 Search and retention

- [x] Implement an independent seeded random-residual sampler as the minimum
      baseline.
- [x] Implement one finite-sample distribution optimizer over the same residual
      surface. Start with CEM; add CMA-ES, ARS, or another method only if a
      measured failure justifies it.
      `dusklight-search::residual_optimizer` implements independent seeded
      random sampling and integer-probability, rank-only categorical CEM over
      the same sealed finite search space. Both compile the authoritative
      `ResidualCandidate` before dispatch and deduplicate on realized raw tape;
      snapshots retain RNG, distribution, pending-genome, and seen-tape state.
- [x] Retain every terminal success inside the exploration horizon, including
      routes slower than the incumbent.
- [x] Rank successes by deterministic first-hit tick, then tape simplicity and
      declared risk. Do not let a shaped reward override that order.
- [x] Keep failures as experience and diversity evidence. Do not pretend that
      Euclidean distance to the exit is terminal success or universal progress.
- [x] Maintain successful trajectory diversity long enough for coordinated
      alternatives to survive; do not collapse immediately to one greedy elite.
- [ ] Tighten the exploration horizon only after the retained successful basin
      supports it.
- [ ] Minimize a winner only after discovery, using exact terminal replay as the
      acceptance authority.
      The sealed `residual_retention` archive now implements these policies,
      including candidate/tape-bound exact evidence, deterministic
      first-hit/simplicity/risk ordering, diverse success elites, a failure
      diversity reservoir, supported horizon-tightening evidence, and
      replay-authoritative post-discovery minimization. Its complete history
      round-trips through a sealed resume snapshot. The persistent native runner
      now validates every exact terminal result, records it through this archive,
      uses the archive's deterministic generation rank for CEM, and checkpoints
      the complete archive with the optimizer. Horizon tightening and winner
      minimization remain separate post-discovery operations below.

### 2.4 Baseline proof

- [ ] First improve a deliberately degraded version of the Ordon tape whose
      removable inefficiency is known but not encoded in the optimizer.
- [ ] Run independent random and CEM residual campaigns against the real 125-tick
      incumbent under sealed budgets.
- [ ] Report successful-episode rate, unique compiled tapes, action/window
      coverage, first-hit distribution, retained basin diversity, and improvement
      by simulated tick.
- [ ] Promote any sub-125 winner only after five identical cold replays.
- [ ] If no winner appears, preserve enough evidence to distinguish exhausted
      residual coverage from a broken generator, truncated budget, absent
      successes, or premature population collapse. Do not call 125 optimal.

**Gate 2:** the residual optimizer improves the degraded canary and operates as a
credible, observable search on the 125-tick incumbent. A deterministic sub-125
winner is the target and remains an open framework challenge until achieved.

## 3. Close the autonomous learning loop

- [x] Add one campaign command/API that owns persistent workers for the entire
      run instead of relaunching the game per generation.
      `campaign run-residual-optimization --execution EXECUTION.json` launches
      one persistent native checkpoint session per sealed worker lane, reuses it
      across every generation, verifies pool build identity, and shuts all
      sessions down on success or error.
- [ ] Automatically ingest all eligible native episodes into immutable replay
      generations with demonstration, policy-rollout, randomized-coverage, and
      alternate-terminal roles.
- [ ] Train a goal-conditioned estimate of terminal reachability and time-to-go
      from complete trajectories using tick cost, real terminal outcomes, n-step
      returns, replay, target isolation, and uncertainty.
- [ ] Export the trained policy into the native frozen format without a manual
      translation step.
- [ ] Execute the frozen policy online from pre-input state, ingest its realized
      episodes, refit, and repeat until the declared budget or stopping rule is
      reached.
- [ ] Preserve policy, dataset, checkpoint, objective, feature, action, proposal,
      and parent-generation lineage across the loop.
- [ ] Seed optimization runs with the incumbent demonstration while allowing the
      policy to leave its trajectory.
- [ ] Build a reverse curriculum from validated states on actual successful
      trajectories.
  - Begin close enough to the terminal predicate to observe several viable
    continuations.
  - Expand backward only after held-out rollouts reconnect the new frontier to a
    terminal success.
  - Never replace reachability with a hand-authored coordinate corridor.
- [ ] Use residual search for continuous trajectory proposals and the learned
      critic for state-conditioned ranking. Keep exact simulation as authority.
- [ ] Preserve achieved goals and alternate terminals so failed main-goal runs
      still teach local dynamics and short-horizon reachability.
- [ ] Detect and report policy collapse by parent diversity, action diversity,
      state/contact coverage, and success distribution.
- [ ] Stop or fall back explicitly when held-out policy performance fails. The
      UI and artifacts must identify which proposal source actually ran.

**Gate 3:** one unattended command performs at least three real
collect/train/freeze/native-execute/ingest generations in persistent workers,
survives resume, and emits cold-replayable candidates without manual artifact
conversion.

## 4. Prove that learning adds value

- [ ] Freeze equal simulated-tick budgets and identical initial-state
      distributions for:
  - independent random residual search;
  - CEM residual optimization;
  - learned state-conditioned proposals combined with the same residual surface.
- [ ] Evaluate across several deterministic seeds and held-out checkpoints.
- [ ] Require the learned method to improve successful-episode rate, best
      first-hit time, or sample efficiency over the non-learning baselines.
- [ ] Add negative controls for shuffled outcomes, action-only input, removed
      collision/geometry, removed actors, removed history, and checkpoint/tape
      identity leakage.
- [ ] Attribute failure before changing architecture:
  - observation insufficiency;
  - lossy or unsupported action surface;
  - sparse terminal coverage or poor credit assignment;
  - exploration/population collapse;
  - native/offline inference mismatch;
  - insufficient simulation throughput.
- [ ] Add or change FQI, Double Q, DDQN, recurrent state, model-based rollouts,
      or network capacity only when a controlled comparison targets one of those
      diagnosed failures.
- [ ] Track terminal success, time-to-go calibration, critic disagreement,
      effective state/action coverage, and performance by checkpoint. Never
      promote on training loss alone.

**Gate 4:** under equal budgets, state-conditioned learning produces more held-out
successes, faster valid routes, or materially better sample efficiency than the
continuous optimizer, and appropriate negative controls remove the advantage.

## 5. Make the loop usable from the Route Workbench

- [x] Expose one optimization action for a selected segment/goal. The browser
      submits a typed request; Rust remains the execution and validation
      authority.
- [x] Provide sensible defaults so ordinary operation does not require choosing
      an algorithm or editing a generated request.
- [x] Show campaign state: queued/running/completed candidates, generations,
      simulated ticks, successes, current best, proposal sources, workers, and
      explicit blockers.
- [x] Project generated candidates as ordinary ephemeral sibling segments while
      the campaign runs. Refreshing must add completed candidates and remove
      deleted campaign artifacts.
- [x] Let any candidate play through the normal playback path and acquire a
      thumbnail without a separate search workflow.
- [ ] Make promotion an explicit Git-owned operation that stores the compact tape,
      exact proof, parent boundary, and lineage. Generated failures and discarded
      candidates remain outside source control.
- [ ] Support cancel, resume, and cleanup without leaving native workers, hidden
      windows, locks, or orphaned artifacts.
- [ ] Keep the workbench responsive while thousands of candidates are generated;
      summarize campaigns and load candidate detail on demand.

**Gate 5:** from one selected route segment, a user or agent can start, observe,
stop, resume, inspect, replay, and promote an optimization result without manually
assembling CLI stages or moving artifacts.

## 6. Demonstrate goal-only discovery separately

This is not a prerequisite for the first usable demonstration-seeded optimizer.

- [ ] Start from the same Link-control checkpoint and terminal predicate without
      the incumbent tape, incumbent-relative residuals, path coordinates, or
      route-progress features.
- [ ] Supply generic world observations, the complete factorized action surface,
      checkpoint-backed curriculum, intrinsic coverage, and hindsight goals.
- [ ] Preserve diverse spatial, contact, action-phase, event, and actor-relative
      states rather than one distance-minimizing frontier.
- [ ] Produce a terminal success and export its exact raw tape.
- [ ] Cold-replay that tape from ordinary boot and require identical per-tick
      gameplay.

**Gate 6:** the system discovers a deterministic Ordon Springs route from the
goal and world state alone. It need not initially beat the optimized route.

## 7. Capability backlog driven by measured failures

These capabilities matter, but none should displace Gates 1-3 without evidence
that it blocks the current loop.

### Native observations and encoders

- [ ] Compile the selected complete actor-set, collision, geometry, surface-graph,
      event/loading, and optional history views into bounded native online
      encoders with exact Rust/native parity.
- [ ] Compare complete-set encoders against any bounded/truncated alternative on
      held-out outcomes before accepting truncation.
- [ ] Finish machine-readable stage/profile coverage for fields actually selected
      by a promoted learner.
- [ ] Extend actor-family state, RNG/timers, resource loading, interaction, or
      proof channels when a neutral coverage audit or benchmark identifies a
      concrete missing signal.

### Stateful tactics

- [ ] Allow a policy to invoke bounded tactics through the native input boundary
      while recording every generated PAD frame and read-only query.
- [ ] Begin with generic control primitives only: maintain relative heading or
      offset, seek a coordinate or portable actor identity, compose a short curve,
      control camera while moving, and synchronize a button edge with an observed
      action phase.
- [ ] Treat tactics as optional action parameterizations, never gameplay writes or
      terminal authorities.
- [ ] Learn or mine useful tactic initiation/termination conditions from
      experience instead of embedding route coordinates or published glitch
      procedures.

### Narrow-basin discovery

- [ ] Add a quality-diversity archive over relational position, contact/surface,
      velocity, action phase, actor/item relationships, event changes, and novel
      displacement.
- [ ] Adapt archive resolution around empirically sensitive dimensions so rare
      precision basins survive without globally chosen floating-point epsilons.
- [ ] Combine learned setup, locomotion, interaction, and frame-synchronization
      behavior with short-horizon continuous/discrete boundary refinement.
- [ ] Require an exact input-only deterministic proof for every claimed outcome.

### Broader information coverage

- [ ] Continue the bootable-world and complete-actor survey as background coverage,
      not as a prerequisite for controlling Link in Ordon.
- [ ] Use Skybook and other catalogs only to identify reusable missing observation,
      action, history, and proof capabilities. Do not translate published setups
      into learner routes or rewards.
- [ ] Validate new channels with neutral temporal coverage, exact replay, and
      explicit missingness before exposing them to a learner.

## Definition of usable

The first usable release is complete when:

- the repository builds cleanly at its recorded submodule revisions;
- one command or workbench action launches a resumable persistent campaign;
- exploration horizon is independent of promotion threshold;
- candidates use precise factorized raw PAD and coordinated continuous residuals;
- slower successes, failures, and winners all enter authenticated experience;
- a trained frozen policy executes natively and round-trips through exact Rust
  reinference and cold tape playback;
- at least three autonomous learning generations run without manual conversion;
- the residual baseline improves a known-degraded canary;
- Ordon 125 receives a measured continuous optimization campaign, with any winner
  promoted only after five identical cold replays;
- generated candidates appear in the existing Route Workbench and can be played,
  inspected, promoted, or discarded through the normal segment workflow;
- every failure reports whether it came from determinism, build, observation,
  action coverage, proposal allocation, evaluation, learning admission, or an
  exhausted declared budget.

## Explicitly deferred

- Adding more named RL algorithms without a controlled failure requiring them.
- Exhaustively proving every actor field on every map before the first closed
  control loop.
- Manually reproducing Skybook glitches or encoding their published solutions.
- Treating route-specific waypoints, wall-following instructions, or distance to
  the Ordon exit as learning success.
- A new learning-specific visualization application; the Route Workbench should
  display ordinary generated segments.
- Scaling workers or model size to compensate for missing successful experience,
  lossy actions, broken budgets, or inference mismatch.
- Coupling this execution plan to the separate causal route-planner project.
