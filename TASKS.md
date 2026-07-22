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
- Eligible exact successes can now be promoted explicitly from the Workbench.
  Promotion trims the compact realized tape to the authenticated hit, performs
  five fresh headless cold replays from boot against the sealed execution
  binding, requires an identical exact terminal boundary on every run, then
  installs the tape, sealed proof, parent boundary, goal, and a new continuation
  lineage as one stale-checked Git-owned timeline edit. Failed verification and
  discarded candidates remain outside source control.
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
- Campaign lifecycle controls now stop through a cooperative token observed at
  durable batch/checkpoint boundaries, report cancellation only after persistent
  native workers are shut down, and remove each run's ephemeral state/cache tree
  on normal return or unwind. Cancelled campaigns resume from sealed evidence;
  stale-bound cleanup removes only the selected physical `build/campaigns/`
  root, rejects symlink escapes and active promotion, and cannot race start,
  cancellation, or promotion registration.
- Workbench campaign polling remains summary-first and projects at most eight
  ranked successes plus the newest completed rows, capped at sixteen ordinary
  sibling nodes even with ten thousand candidates. Full execution, candidate,
  residual, evaluation, artifact, terminal-boundary, and per-attempt native
  evidence is served by a stale-bound authenticated endpoint and loaded only
  when the selected candidate asks for detail; deleted candidates evict cached
  browser detail on refresh.
- Persistent residual campaigns now decode and authenticate every completed
  native episode shard into a cumulative, content-addressed replay generation
  before advancing their optimizer checkpoint. Checkpoint v3 binds constant-size
  replay episode/transition/outcome counts for responsive workbench polling;
  retries reuse identical corpus bytes, pre-v3 completed campaigns backfill on
  resume, and residual proposals are explicitly classified as randomized
  coverage rather than demonstrations or policy rollouts.

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
- [x] Tighten the exploration horizon only after the retained successful basin
      supports it.
      `campaign tighten-residual-horizon` now derives a child optimization
      request from the retained exact-success archive under an explicit minimum
      success, behavior-class, and support-fraction policy. The child binds the
      source request, native execution, and checkpoint by digest; recursively
      revalidates that lineage; recomputes the basin evidence; and permits only
      a fresh request ID, the supported lower horizon, its conservative derived
      tick budget, fresh resume paths, and the sealed lineage record to change.
      A resealed request with any other delta fails file validation.
- [x] Minimize a winner only after discovery, using exact terminal replay as the
      acceptance authority.
      The sealed `residual_retention` archive now implements these policies,
      including candidate/tape-bound exact evidence, deterministic
      first-hit/simplicity/risk ordering, diverse success elites, a failure
      diversity reservoir, supported horizon-tightening evidence, and
      replay-authoritative post-discovery minimization. Its complete history
      round-trips through a sealed resume snapshot. The persistent native runner
      now validates every exact terminal result, records it through this archive,
      uses the archive's deterministic generation rank for CEM, and checkpoints
      the complete archive with the optimizer.
      `campaign minimize-residual-winner` now admits only a candidate that
      recompiles to an original retained exact success in a digest-bound source
      request/execution/checkpoint chain. It performs bounded deterministic
      component ddmin through the persistent validated checkpoint pool, accepts
      only strict compiled-tape simplifications whose exact native repetitions
      agree and remain no slower than the discovery, and retains every evaluated
      reduction, batch/result/episode proof, tick charge, and reproduced archive
      snapshot in a sealed independently revalidatable summary. A sealed request
      makes partial runs safely resumable; no shaped score can accept a
      reduction. Horizon tightening and minimization remain separate explicit
      post-discovery operations.

### 2.4 Baseline proof

- [ ] First improve a deliberately degraded version of the Ordon tape whose
      removable inefficiency is known but not encoded in the optimizer.
- [ ] Run independent random and CEM residual campaigns against the real 125-tick
      incumbent under sealed budgets.
- [ ] Report successful-episode rate, unique compiled tapes, action/window
      coverage, first-hit distribution, retained basin diversity, and improvement
      by simulated tick.
      New v4 campaign checkpoints now carry an incrementally reproduced,
      content-sealed audit over the exact evaluation-journal order. It reports
      zero-inclusive raw-PAD action, temporal-basis, and intervention-octant
      coverage; main-terminal episode rate and first-hit distribution; unique
      realized tapes and successful behavior classes; separately accumulated
      invalid-genome and duplicate-tape rejection plus CEM concentration
      evidence; and every strict sub-incumbent improvement at its
      cumulative simulated-tick charge. The CLI completion summary returns the
      audit and the Route Workbench projects its bounded diagnosis and coverage.
      Legacy v2/v3 checkpoints remain readable and migrate to v4 at the next
      durable checkpoint. The real degraded-canary and Ordon runs must still
      populate the final comparative reports before this empirical item closes.
- [ ] Promote any sub-125 winner only after five identical cold replays.
- [ ] If no winner appears, preserve enough evidence to distinguish exhausted
      residual coverage from a broken generator, truncated budget, absent
      successes, or premature population collapse. Do not call 125 optimal.
      The checkpoint audit now distinguishes declared-budget completion,
      tick-budget truncation, proposal-generation stalls, exact successes without
      improvement, and no-success completion while retaining attempted-genome,
      invalid-versus-duplicate rejection, categorical-concentration, coverage,
      and basin facts.
      A no-winner campaign remains required before this conditional evidence can
      be judged in practice.

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
- [x] Automatically ingest all eligible native episodes into immutable replay
      generations with demonstration, policy-rollout, randomized-coverage, and
      alternate-terminal roles.
      Native residual random/CEM attempts now ingest automatically as authenticated
      randomized-coverage generations and are checkpointed atomically with the
      optimizer. The unattended goal loop now admits independently reinferred
      frozen-policy v3 shards as policy-rollout generations with exact manifest
      lineage. Fresh optimization runs now authenticate the incumbent's exact
      native success as a demonstration generation before proposing candidates.
      Optimization requests now also seal a sorted set of same-segment,
      route-proved, history-free post-simulation alternate goals. Every failed
      main-terminal candidate is rerun against those authored predicates through
      dedicated persistent checkpoint pools; all rerun ticks are journaled, and
      exact alternate hits enter the same cumulative corpus under their own
      objective and `alternate_terminal` role. Main-terminal successes retain
      their authoritative outcome rather than being reclassified. Replay
      checkpoints authenticate every role back to its native shard and reject
      missing, extra, failed, or request-detached terminal experience.
- [ ] Train a goal-conditioned estimate of terminal reachability and time-to-go
      from complete trajectories using tick cost, real terminal outcomes, n-step
      returns, replay, target isolation, and uncertainty.
      An immutable goal-trajectory dataset now authenticates replay entries back
      to complete native shards and a compiled semantic goal, keeps whole episodes
      in one deterministic split, binds the exact native feature/action schemas,
      and materializes time-to-goal, fixed-point tick cost, Monte Carlo outcome,
      and linked n-step bootstrap targets. `huntctl learn goal-trajectory-dataset`
      writes the content-addressed artifact without copying observations. The
      Rust-native `fit-goal-reachability` ensemble now joins those rows back to
      authenticated pre-input observations, conditions on a digest-free semantic
      goal graph embedding, fits training-only normalization, replays whole
      episode bootstraps, and uses frozen epoch targets for return and tick cost.
      Validation—not test—gates all four real-outcome, time, return, and cost
      heads against training-mean baselines plus an uncertainty ceiling; test is
      reported only after that decision. The sealed model preserves dataset,
      replay, goal, observation, feature, and action lineage and has no promotion
      authority. A real campaign corpus still needs to satisfy the split gate and
      produce an admitted model before this item is complete.
- [x] Export the trained policy into the native frozen format without a manual
      translation step. `huntctl learn fit-frozen-goal-policy` now authenticates
      successful trajectory actions back to native pre-input observations,
      requires an admitted reachability model for the exact dataset and goal,
      trains and admits a state-conditioned factorized PAD policy on whole-episode
      train/validation/test splits, folds training-only normalization into its
      first layer, and writes the existing C++-readable `.dsfrozen` format
      directly. Its separately sealed manifest preserves dataset, replay, critic,
      goal, feature, action, split-episode, training, metric, and frozen-byte
      lineage; `inspect-frozen-goal-policy` verifies both artifacts together.
- [ ] Execute the frozen policy online from pre-input state, ingest its realized
      episodes, refit, and repeat until the declared budget or stopping rule is
      reached. The persistent native worker boundary now accepts exact frozen
      policy v5 batches as well as residual batches, pins every refit to the same
      checkpoint and authored terminal, validates v6 model/timing/episode
      identities, requires policy-tagged v3 shards, and independently reinfers
      every emitted PAD in Rust before admission. A sealed learning-loop request
      and crash-safe JSONL state machine now preserve prepared, executed, and
      replay-committed phases across at least three ordered generations, account
      every rollout tick, recover a torn tail, and reject artifact, parent-corpus,
      phase, or stopping-rule tampering. `campaign run-native-goal-learning-loop`
      now trains from the active corpus, freezes directly, fans each generation
      across persistent checkpoint workers, adopts valid crash leftovers,
      independently reinfers every PAD, ingests policy-role replay, refits, and
      materializes each consumed policy suffix as an ordinary cold-replayable
      process-boot tape before stopping explicitly on held-out rejection or its
      sealed limit. A real admitted campaign must still complete three generations
      before this item and Gate 3 are proven complete.
- [x] Preserve policy, dataset, checkpoint, objective, feature, action, proposal,
      and parent-generation lineage across the loop. The loop request binds the
      optimization proposal and native execution authorities; each journal phase
      binds corpus, dataset, critic, manifest, model, batch, result, reinference,
      checkpoint-restored shard, and next-corpus identities in order.
- [x] Seed optimization runs with the incumbent demonstration while allowing the
      policy to leave its trajectory. A fresh native campaign now reproduces the
      sealed incumbent once through its ordinary persistent checkpoint worker,
      authenticates the exact successful episode in a content-addressed
      demonstration manifest, charges its simulated ticks in the crash-safe
      resume journal, and installs it as generation one of cumulative replay
      before any candidate is proposed. Random/CEM residual proposals and frozen
      online policies remain free to emit every factorized PAD action rather than
      being constrained to the demonstration. Legacy pre-seed checkpoints remain
      resumable, while new seeded checkpoints, later replay generations, and the
      workbench learning launch fail closed if the demonstration or its shard is
      missing or detached. The workbench identifies whether the seed exists and
      shows its exact tick charge.
- [ ] Build a reverse curriculum from validated states on actual successful
      trajectories.
  - Begin close enough to the terminal predicate to observe several viable
    continuations.
  - Expand backward only after held-out rollouts reconnect the new frontier to a
    terminal success.
  - Never replace reachability with a hand-authored coordinate corridor.
      `campaign seed-residual-reverse-curriculum` now derives a narrow terminal
      action window from the exact route-proved incumbent prefix; no coordinate
      or synthetic-state corridor enters the request. Each child is sealed to
      its source request and owns fresh resume paths. `campaign
      expand-residual-reverse-curriculum` moves that window backward by exactly
      one policy step only after a digest-bound native checkpoint contains the
      configured number, behavior diversity, and rate of exact successful
      continuations. Recursive file validation rebuilds every seed/expansion and
      rejects any other request delta. Real held-out curriculum campaigns must
      still populate and expand this lineage before the item closes.
- [ ] Use residual search for continuous trajectory proposals and the learned
      critic for state-conditioned ranking. Keep exact simulation as authority.
- [x] Preserve achieved goals and alternate terminals so failed main-goal runs
      still teach local dynamics and short-horizon reachability. The checked
      Ordon campaign now binds `ordon_spring_exit_approach` as an alternate to
      the committed-load promotion terminal and reserves the full exact-simulation
      budget for both evaluations. Mixed-objective replay retains each authored
      objective identity and successful short horizon, while one
      goal-conditioned dataset selects only entries matching its compiled goal;
      alternate experience remains available for sibling goal datasets and has
      no promotion authority. The workbench shows configured alternate goals and
      the authenticated alternate-hit count and resolves their shards for the
      unattended learning loop.
- [x] Detect and report policy collapse by parent diversity, action diversity,
      state/contact coverage, and success distribution. Every committed online
      generation now writes a content-sealed collapse report over its realized
      native shards. Journal replay independently recomputes parent-state,
      consumed-action/trajectory, state-identity, contact-signature, and exact
      terminal-outcome diversity and rejects detached reports or success counts.
      CLI completion summaries identify warning generations and the latest
      warning set; the route workbench shows the exact counts, warnings, and
      report artifact without treating the diagnostics as simulation authority.
- [x] Stop or fall back explicitly when held-out policy performance fails. The
      UI and artifacts must identify which proposal source actually ran. The loop
      journal and CLI summary now stop with `retained_baseline` when reachability
      or policy admission fails. The route workbench now launches, resumes, and
      cancels the sealed loop, projects generation/tick/corpus progress and
      cold-replayable tape counts, and distinguishes `frozen_goal_policy` from
      `retained_baseline` with the exact stopping reason.

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
- [x] Make promotion an explicit Git-owned operation that stores the compact tape,
      exact proof, parent boundary, and lineage. Generated failures and discarded
      candidates remain outside source control.
- [x] Support cancel, resume, and cleanup without leaving native workers, hidden
      windows, locks, or orphaned artifacts.
- [x] Keep the workbench responsive while thousands of candidates are generated;
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
