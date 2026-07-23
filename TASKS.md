# Active task: ship a usable deterministic TAS learning and optimization loop

## Outcome

Given an exact source boundary, an authored terminal predicate, and an optional
human demonstration, one unattended campaign must be able to:

1. restore the exact source state without replaying unrelated history;
2. give a state-reactive policy complete controller authority for the entire
   episode rather than confining it to edits around an incumbent;
3. execute exploration through persistent native workers without rendering or host
   pacing affecting simulation;
4. retain slow successes, failures, and circuitous trajectories as learning
   experience under a generous or adaptive horizon;
5. treat a human tape as optional replay/curriculum evidence, never as the
   coordinate system of the learner's action space;
6. improve its proposals across generations, then use incumbent-relative
   residual search only as a distinct route-refinement stage;
7. expose live candidates as ordinary ephemeral siblings in the Route Workbench;
8. promote a winner only after exact cold replay from boot.

The first benchmark is the checked 125-tick `ToOrdonSprings` segment. It is a
small control problem intended to expose framework defects cheaply. It is not a
claim that Ordon-specific machinery is useful elsewhere.

The deliberately degraded 131-tick Ordon tape is a hard infrastructure canary:
the allowed refinement surface must first be shown capable of expressing its
known repair, and an optimizer must recover at least the known 125-tick result
without being supplied that repair. Separately, a discovery campaign must be
able to learn a successful route with episode-long action authority. Passing one
regime does not stand in for the other.

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
  promotion strictly sub-125. That request is now classified strictly as a
  local residual-refinement experiment: its horizon and incumbent-relative
  proposal surface are not evidence of route discovery.
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
  Fresh materialization also preserves the normal ignored repository-relative
  disc-image symlink, authenticates its external target bytes through the shared
  large-artifact digest cache, and rejects direct external paths or any
  intermediate-directory symlink escape. The checked Ordon request can once
  again seal a current execution binding against that local layout.
- The repaired macOS Ordon source chain reaches the native Link-control boundary
  at frame 506, validates an eight-tick checkpoint replay there, and reproduces
  the incumbent's F_SP104 load commit at raw suffix boundary 125 after 126
  sampled transitions. The live q125 CEM campaign has sealed that incumbent as
  its demonstration and prepared generation zero. Three of its four native
  lanes completed, and a fresh resume authenticated and adopted those three
  result/shard pairs before launching only the missing lane, proving the
  crash-window adoption path against real campaign evidence.
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
- Native checkpoint restore now rewinds the host-side JKR volume, ARAM, and DVD
  registries with their emulated-memory links. A fresh four-lane degraded-canary
  run crossed the former `dMeterButton_c::screenInitButton` crash repeatedly.
  Candidate episode evidence now binds the authenticated member ID as well as
  its shared batch shard, so all 64 results in a generation can enter retention
  without falsely appearing to reuse one episode for different tapes.

Not yet working:

- There is no closed `collect -> train -> freeze -> execute -> ingest -> refit`
  campaign.
- There is no campaign in which a learner receives full state-reactive PAD
  authority throughout a genuinely exploratory episode. The checked residual
  requests stop at tick 160, permit at most four incumbent-relative edits, and
  restrict intervention starts to frames `0..126`; they cannot establish broad
  learning or route discovery.
- Campaign artifacts do not yet identify and enforce three distinct experiment
  classes: demonstration-assisted discovery, from-scratch discovery, and local
  TAS refinement.
- The 125-tick Ordon incumbent remains unbeaten.
- The degraded q131 canary remains unimproved by independent random or CEM
  sampling. A transient v3 run did reach tick 125, but its first population slot
  was the already known one-frame repair at frame 100. The checked source no
  longer contains that generation-zero injection, the report pointed at a
  different request revision, and its executable identity disagreed with the
  referenced execution binding. That run is rejected as search evidence in
  `docs/glitch-hunting/benchmarks/ordon-degraded-q131-recovery-20260722.json`.
- The historical 18,867-candidate Ordon campaign was local procedural mutation,
  not evidence of successful learning.
- The current learned proposer is an offline window-patching system over a coarse
  action catalog, not a continuous state-reactive policy.

Measured and structural reasons the Ordon work has not yet demonstrated useful
optimization or learning:

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
- The current 160-tick horizon allows only 29-35 ticks beyond the incumbents,
  and the residual action window ends before that horizon. This is acceptable
  for a deliberately local baseline but prevents substantially slower routes
  from becoming useful discovery experience.
- The incumbent is still the substrate of every residual proposal. Retaining it
  as one demonstration is valid; requiring every candidate to be its bounded
  perturbation is not a learning experiment.
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
  may learn from much slower successes while promoting only a faster result. A
  discovery horizon must be justified by goal reachability or an adaptive
  curriculum, never by a small multiplier over the incumbent.
- Every campaign declares whether it is demonstration-assisted discovery,
  from-scratch discovery, or incumbent-relative refinement. Reports never
  compare or conflate those regimes.
- A demonstration may seed replay, behavior cloning, or checkpoint curriculum.
  It must not restrict the actions, coordinates, temporal windows, or trajectory
  neighborhood available to a discovery policy.
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
- [x] Separate the first Ordon residual campaign's exploration horizon from its
      promotion threshold so it can retain locally perturbed slower successes.
      Promotion remains strictly sub-125.
      The sealed `ordon-q125-residual-campaign.request.json` explores through
      tick 160 while retaining the incumbent first-hit tick and strict
      `promotion_before_tick = 125` as separate authenticated fields. Its v2
      source now includes the console-faithful TV calibration screen, binds the
      Link-control checkpoint at frame 506 to native fingerprint
      `e7ac8251329f22a5df682bbe5eb2a2ba`, and passes an exact 8-tick checkpoint
      restore plus the incumbent's tick-125 F_SP104 load commit. Its 160-tick
      horizon is a local-refinement parameter and must not be reused as the
      default for unrestricted learning.
- [x] Make campaigns resumable after cancellation, worker crash, or UI closure
      without repeating sealed candidates or losing optimizer state.
      `native_residual_campaign_runner` now journals candidate batches before
      dispatch, revalidates and adopts complete result artifacts after a crash,
      allocates a fresh result path around partial artifacts, records every
      evaluation before the optimizer update, and restores the exact pending
      CEM generation. Focused tests cover deterministic residual-to-native
      conversion at frame 506 and non-overwriting crash-window recovery.

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

- [x] Produce one exact hand-authored repair of the degraded q131 tape using only
      the residual operations allowed by its sealed search space. Keep the
      witness out of optimizer initialization; it proves expressiveness, not
      search competence. A focused regression seals one ordinary one-frame
      `0x0100` button residual at suffix frame 100, compiles it against the
      checked q131 tape, and obtains the checked q125 bytes exactly. The witness
      is absent from CEM generation construction, replay, ranking, and proposal
      distributions.
- [x] Show that independent sampling over the declared residual surface produces
      meaningful variation in successful first-hit times. If it does not, fix
      the action surface or temporal bases before tuning CEM.
      The sealed equal-budget q131 random v3 campaign completed all 1,024 unique
      tapes at checkpoint identity
      `c6ada18be5e75a99353482a8da638ceb30adc54bb2c2e6e489431840ffe558f7`.
      Its 681 exact successes range from tick 130 through tick 143 across 332
      behavior classes, with nonzero coverage for button press/release, both
      main-stick axes, both camera axes, all eight start octants, and every
      declared exact/window/ramp/curve temporal basis. Independent sampling
      therefore exercises a meaningfully variable success basin rather than a
      terminal predicate or action surface collapsed to one hit time.
- [x] First improve a deliberately degraded version of the Ordon tape whose
      removable inefficiency is known but not encoded in the optimizer.
      The sealed q131 CEM v5 campaign completed its declared 1,024-candidate
      budget at checkpoint content identity
      `d09df73560542f250eeb5b4bd336a4fcd606d6d2145ca76aaca1a9180847e6b1`.
      All 1,024 compiled tapes were unique; 760 candidates succeeded, spanning
      426 successful behavior classes, for a 742,187-millionths successful
      episode rate. Candidate
      `5eb2132cf8e4c06efc9f2223e72cd92eddf4ba833d308cf8216f09bf25c77250`
      first hit the exact terminal at tick 128 after 75,759 charged simulated
      ticks, improving the deliberately degraded tick-131 incumbent without
      receiving the checked q125 expressiveness witness as initialization.
      Its realized tape, behavior, and episode identities are respectively
      `9f7fa710016624d2d920d5c7397eb34f427a86fe74aea665e658cc1438cc5e94`,
      `513de318b95c534d0626459bac9588235778dd23791944c0b80b9a1d3ab75d01`,
      and `2184d3a20b7932249c5b71e22259eca12e39e583c7b1b185942e24d32557764c`.
      The complete audit charged 185,113 ticks including the 132-tick
      authenticated incumbent demonstration and diagnoses `winner_found`.
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
      The degraded-canary half is now populated by equal 1,024-candidate random
      and CEM audits. Random achieved 665,039-millionths success, 332 successful
      behavior classes, best tick 130, and first improved at 149,565 charged
      ticks; CEM achieved 742,187-millionths success, 426 classes, best tick 128,
      and first improved at 75,759 charged ticks. Both produced 1,024 unique
      tapes and broad nonzero action/window coverage. Their complete audit
      identities are respectively
      `1d589a710c5f86c731a34881ebd5c23f1d351955447b14f4c09b08d9dc7a8509`
      and `6ecaefbef4f123233f860c3e13dfa476d25736faa1f22aaf2985a6ee7db2aa9a`;
      the real q125 comparison remains required before this item closes.
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
Failure to recover the known q125 behavior is an infrastructure failure, not a
reason to describe the incumbent as optimal or proceed to a more elaborate
learner.

## 3. Close the autonomous learning loop

This gate is demonstration-assisted discovery, not residual optimization. The
incumbent may contribute one authenticated trajectory, behavior-cloning seed, or
reverse-curriculum state sequence, but the online policy must be able to ignore
it and emit any legal PAD action for the entire episode.

- [x] Encode the campaign class explicitly and reject a request that labels an
      incumbent-relative proposal surface as discovery. Optimization requests
      now identify `local_tas_refinement`; native goal-learning loops identify
      `demonstration_assisted_discovery`; the shared closed enum reserves
      `from_scratch_discovery`. Residual-request validation rejects either
      discovery label, and the learning loop rejects a non-refinement source
      authority. Checked requests and validation reports carry the class.
- [x] Give the native policy episode-long state-reactive action authority. Do not
      end its proposal window at the demonstration's terminal frame or fall back
      to released incumbent input after that point. Frozen-policy v6 batches now
      seal `episode_policy` authority independently of demonstration mode and use
      the exploration horizon rather than the demonstration hit. Native execution
      performs fresh pre-input inference, decodes and consumes that PAD on every
      nonterminal episode tick, and its result authenticates the exact policy-
      controlled tick count plus zero fallback ticks; Rust rejects any missing,
      shortened, or fallback authority evidence.
- [x] Use a generous initial horizon or a success-supported adaptive curriculum.
      Retain slow successes and timeouts as experience; contract the horizon only
      after held-out goal reachability is reliable. Goal-learning validation now
      requires an untightened fixed horizon at least 16 ticks and 10% beyond the
      promotion terminal, reports the exact actual/minimum horizon and timeout-
      retention authority, and rejects residual horizon-tightening lineage as an
      initial discovery horizon. Every full-horizon miss and every slower exact
      success remains a policy-rollout shard in cumulative replay; the learning
      loop performs no horizon contraction, so contraction can only enter through
      a separately sealed, support-gated authority after this fixed campaign.
- [x] Support controlled demonstration modes: absent, replay-only, behavior-
      cloning warm start, and reverse-curriculum checkpoints. Record the active
      mode in every model, rollout, and comparison artifact. The sealed loop
      exposes all four treatments: `absent` forbids demonstration entries and
      classifies the run as from-scratch discovery; `replay_only` admits their
      trajectories to reachability training but excludes their PAD targets from
      policy fitting; behavior cloning is the explicit warm start; and reverse
      curriculum requires both demonstration entries and a sealed curriculum
      checkpoint authority. Dataset, reachability model, frozen-policy manifest,
      strict native rollout envelope, collapse comparison, and run summary all
      bind the selected mode.
- [x] Supply a stable generic observation contract containing Link motion/action
      state, camera/PAD history, local collision contacts and geometry, target
      trigger relation, loading/event state, and complete relevant actor sets
      with explicit missingness. The content-hashed generic observation v1
      contract fixes learning-observation v27 plus raw-PAD v2, requires bounded
      past-only complete-observation/action history, and rejects unsampled camera,
      player-action, collision, scene-exit, lifecycle, event-transition, room,
      warp, resource, or relationship channels. It also requires untruncated
      runtime-generation actor sets and finite scene-exit/trigger geometry.
      Learning-loop validation reports the contract, history, observation, and
      actor counts, and every newly executed policy shard must pass it before
      replay ingestion.
- [x] Separate authoritative rewards from diagnostics. Terminal success and
      per-tick cost define the task; navigation-aware potential, novelty,
      contacts, and learned reachability may shape exploration but cannot declare
      success or encode a human waypoint corridor. Goal-trajectory dataset v2
      seals `authored_terminal_and_unit_tick_cost` as its only reward authority;
      every return and bootstrap target is recomputed from authenticated native
      terminal outcomes and duration. Diagnostic shaping reports now carry
      `terminal_objective_unchanged: true` and `promotion_authority: false`, and
      dataset validation rejects any attempt to grant shaping promotion authority.

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
- [x] Train a goal-conditioned estimate of terminal reachability and time-to-go
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
      authority. The real q131 cumulative campaign corpus now admits model
      `c115c3fbda4221e1c7aa7c5d176f6373e90714ff4f02f13ee56443aa765da327`
      as a `goal_conditioned_candidate`. Its 98-episode validation split improves
      reachability Brier by 0.631332, successful time MAE by 0.943045,
      discounted-return RMSE by 0.562950, and discounted tick-cost MAE by
      0.816112 over training-mean baselines, with mean reachability disagreement
      0.029794. The separately reported 110-episode test split improves the same
      four heads by 0.651620, 0.955051, 0.594426, and 0.827435. The model remains
      diagnostic-only and binds dataset
      `b8aeea2eaba9a290b36ad87e89f8f0d7ec56d842f3efb919fde2e4ef0e827faf`
      and replay corpus
      `3de1412e3ab4d1dd75a55c7dbe83b0c61c7a684e4bdc9c7f7aca5d190099efd0`.
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
- [x] Execute the frozen policy online from pre-input state, ingest its realized
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
      sealed limit. The admitted q131 v2 campaign now proves the complete path:
      one unattended command committed three generations and exactly 1,920
      charged native ticks, grew the authenticated corpus from 1,037 entries /
      144,793 transitions at generation 18 to 1,049 / 146,713 at generation 21,
      emitted and independently reinferred twelve cold-replayable policy tapes,
      and stopped with `generation_limit_reached` plus proposal source
      `frozen_goal_policy`. Its three frozen model identities are
      `20a8a0eaa03a674431295018f5732440`,
      `cdb3e450fb8e6de5268c825f4c0586b6`, and
      `9c6a3e8f7a310198b45bf2c3133ae1d5`; the associated terminal behaviors are
      distinct across generations even though the four deterministic repetitions
      within each generation correctly trigger collapse warnings. Generation 3
      retains admitted reachability model
      `0546dae27b63c0595ad830d09b5e97b524fa2ff830bf0b8100e4cc917472ed17`
      and admitted policy manifest
      `bd7c34ea3fcf33cb2ddfdf0bbc713187e41432bdfd2f046acf96cf06765f99e4`.
      The validated 10-record journal is
      `e6847699dc3d22d850fa3631dfbce5b8f301ab4e8caa5ef41c3fbf82cb7e1872`
      and its final state is
      `f20fdaf22213bb2e62edcef37fafe6d164aa393ba9b9b8d27ac26140952a6634`.
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
      before any candidate is proposed. Its native request extends the ordinary
      incumbent tape with released input through the full exploration horizon,
      while the authenticated first terminal hit still fixes the exact charged
      demonstration length. Random/CEM residual proposals and frozen online
      policies remain free to emit every factorized PAD action rather than being
      constrained to the demonstration. Legacy pre-seed checkpoints remain
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
conversion. Passed by the authenticated q131 v2 campaign above: three committed
generations, twelve native policy executions/cold tapes, 1,920 charged ticks,
three distinct fitted frozen models, a generation-limit stop, and a fully
revalidated crash-safe journal/state chain.

## 4. Prove that learning adds value

- [ ] Freeze equal simulated-tick budgets and identical initial-state
      distributions for:
  - independent random residual search;
  - CEM residual optimization;
  - demonstration-assisted state-reactive discovery;
  - from-scratch state-reactive discovery;
  - learned proposals followed by the same post-discovery residual refinement.
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

## 6. Demonstrate from-scratch goal discovery separately

This is not a prerequisite for the first usable demonstration-seeded optimizer.

- [ ] Start from the same Link-control checkpoint and terminal predicate without
      the incumbent tape, incumbent-relative residuals, path coordinates, or
      route-progress features.
- [ ] Supply generic world observations, the complete factorized action surface,
      episode-long native action authority, a generous/adaptive horizon,
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
- discovery campaigns provide episode-long state-reactive PAD authority and do
  not inherit incumbent-relative edit windows or a near-incumbent time cap;
- the human tape is optional authenticated experience, and the active
  demonstration mode is explicit;
- candidates use precise factorized raw PAD and coordinated continuous residuals;
- slower successes, failures, and winners all enter authenticated experience;
- a trained frozen policy executes natively and round-trips through exact Rust
  reinference and cold tape playback;
- at least three autonomous learning generations run without manual conversion;
- one demonstration-assisted learner reaches the Ordon terminal without being
  constrained to residual edits around the demonstration;
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
