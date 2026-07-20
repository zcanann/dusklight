# Active task: build a learning system, then prove it on Ordon

The immediate objective is not another procedural route search:

> Build an integrated learning loop that retains complete experience, learns
> which state and actions predict future success, and uses that knowledge to
> discover and optimize input sequences. Prove it first on the Link-control to
> Ordon Springs segment.

The 125-tick tape is a demonstration and baseline, not the route definition.
The framework must support both learning from that demonstration and attempting
the same goal without demonstration-relative features.

## Invariants

- Identical initial state and consumed PAD must produce identical per-tick
  gameplay state. Any disagreement is a framework bug.
- Normal automation observes state and supplies controller input; it does not
  patch gameplay state. Checkpoints are an acceleration mechanism only.
- Every promoted result is the exact realized PAD sequence and must replay from
  an ordinary cold boot without a policy, checkpoint, or gameplay write.
- Terminal predicates and simulated ticks are authoritative. Auxiliary rewards,
  learned values, novelty, and demonstrations may guide exploration but cannot
  declare success.
- Capture broadly, but present state to learners through versioned structured
  views. Do not flatten native memory, pointers, renderer state, or future
  information into model input.
- Per-tick observation, policy inference, and PAD application remain native.
  Training and batch orchestration may run outside the frame loop.
- Compare methods under the same simulated-tick budget and initial-state
  distribution. Training loss and algorithm names are not results.

## Existing evidence

- The first-Link-control checkpoint has passed repeated restore and cold-prefix
  equivalence tests.
- Native suffix execution evaluated 18,867 candidates and 2,358,375 ticks
  without beating the 125-tick incumbent.
- Those attempts retained terminal endpoints for failures and were generated
  by route-specific mutation families. They are evidence against that search
  method, not a learning corpus.
- The older Q path retained some trajectories, but its inspected campaign used
  only 3,489 transitions from 12 episodes, sparse terminal-only progress, and a
  coarse action catalog. It is a baseline, not a completed learner.
- The native checkpoint batch now emits independently compressed, checksummed
  episode blocks with versioned observation/action schemas. A live two-candidate
  Ordon batch retained and Rust-decoded all 250 failed transitions, including
  camera, Link action/form, scene exits, collision contacts, polygon identities,
  backing codes and resolved planes: 3,301,240 uncompressed bytes became 63,223
  compressed bytes, and both siblings began from the same restored state
  identity.
- Repeating that batch in a fresh automation root exposed a framework bug: the
  process-boot prefix reached frame 440 with fingerprint `5724e31e...ce558`
  instead of `ac7c3278...b5b4`, while the same build and inputs passed when the
  prior isolated memory-card artifact was present. This is a hidden boot-fixture
  dependency to eliminate, not a reason to mine a more "robust" tape.
- The checked-in native-writer golden shard contains both success and failure
  episodes. Cross-language tests reject action-boundary shifts, terminal labels
  in pre-input observations, malformed actor completeness, corruption, and
  phase discontinuity before a shard can enter learner code.
- Learning-observation v4 separates the complete learner actor population from
  the bounded controller hot path. A native encoder test carries 257 actors,
  the checked C++-writer/Rust-reader fixture proves the v4 contract, and both
  sides fail closed on incomplete actor metadata. A live two-candidate,
  250-tick F_SP103 checkpoint batch emitted v4 under the immutable process-boot
  card fixture and passed the same Rust invariant checks.
- Learning-observation v5 adds the exact 256-byte
  `dSv_info_c::mTmp.mEvent` register bank without modifying gameplay code.
  This preserves the console-backed bytes documented at `0x80406F98`,
  `0x80406F99`, and `0x80406F9D`; the older 185-entry temporary-flag view
  collapses register-style `0xff` labels to booleans and cannot prove text
  displacement by itself. A C++-writer/Rust-reader golden shard checks offsets
  0, 1, and 5 byte-for-byte. A live mixed-outcome Ordon batch emitted v5 from
  the same authenticated frame-440 boundary and the isolated Rust ingestion
  boundary accepted every episode with all 256 register bytes present at every
  pre-input and post-simulation observation.
- Milestone language 1.8 exposes that bank as the bounded read-only query
  `event.temporary_byte(INDEX)`, with equality and bit-mask comparisons. The
  checked Rust-compiler/native-evaluator fixture proves exact offsets 0, 1,
  and 5 across the authenticated wire format. It intentionally uses sentinel
  byte values: the Telma/Louise success predicate must be authored from an
  observed successful/control delta, not guessed from Skybook's address list.
- The checked-in Telma/Louise fixture boots directly to `R_SP116`, room 5,
  layer 4, spawn 0 with an explicit post-twilight flag and RNG identity. Two
  fresh cold roots reached its 30-tick stable predicate at tape frame 148 with
  identical state at all 149 compared boundaries and the same v6 fingerprint
  `72c607ff...f27b2`. Native creation evidence confirms both Telma (profile
  627) and Louise (profile 263) are live; no gameplay write occurs after the
  declared tick-zero fixture is applied.
- `huntctl learn inspect-native` now audits authenticated native shards before
  learner ingestion: channel/mask presence and flag-bit variation, constant
  core channels, actor/RNG/collision-set sizes, exact PAD coverage, outcome
  balance, exact payload duplicates, same-source/same-input determinism
  conflicts, and outcome-separating identity candidates. The live Ordon v4
  shard reports 47-48 actors, 28 actor types, 60 exact consumed PAD states, all
  structured mechanics and flag channels present, no truncation or determinism
  conflict, and the expected warning that its two retained candidates are both
  failures.
- A single native Ordon suffix batch from the authenticated frame-440 source
  now retains a known 127-tick load-zone success and a 128-tick neutral-input
  failure through the same episode contract. `huntctl learn inspect-native`
  accepted all 256 transitions and 512 observations with complete 47-48 actor
  sets, two RNG streams, six collision surfaces, exact chosen/consumed PAD
  agreement, both terminal observations, no non-finite values, discontinuity,
  truncation, duplicate trajectory, determinism conflict, or warning. The
  authenticated shard is `d44d8eed...2fa2f`; its success and failure state
  sequences are `95fe103f...ba74` and `8cd6bba5...047e`. The promoted full
  process-boot tape then reached `exit-f-sp103-to-f-sp104` at frame 567 in two
  fresh cold replays without a controller, model, or checkpoint, producing the
  identical v6 boundary fingerprint `545c3127...ca155`.

## 1. Turn every attempt into learning experience

- [x] Extend the native checkpoint batch loop to retain, for every candidate
  tick: pre-action observation, chosen action and exact consumed PAD,
  post-action observation, predicate/event changes, terminal status, remaining
  tick budget, and deterministic state identity.
- [x] Capture failures and successes through the same episode contract. A
  failed attempt must no longer collapse to one terminal X/Z observation.
- [ ] Define a canonical, versioned observation envelope containing:
  - [x] Link motion, facing, action/animation phase, timers, form and relevant
    status;
  - [x] recent controls and camera state;
  - [x] realized background-collision contacts, surface/polygon identity,
    backing metadata and resolved planes/normals;
  - [x] generic local clearance/geometry queries in Link-relative coordinates.
    `learn geometry-view` now joins authenticated native pre/post observations
    to immutable stage inventories entirely offline, retaining bounded nearest
    surfaces and explicit missingness rather than copying a map per tick or
    calling live collision code. The fixed model representation v2 rotates
    closest-point offsets and surface normals into Link space (and actor yaw
    relative to Link) before inference;
  - [x] active gameplay actors as a complete, deterministically ordered
    variable-length set with raw semantic identity fields, transform, velocity
    and type/profile metadata;
  - [ ] derive Link-/camera-/goal-relative actor features and expose available
    typed actor components without changing the canonical raw actor set;
  - [ ] triggers, exits, loading/event state, goal state, clocks and RNG
    identity beyond the currently retained scene-exit and core channels.
- [x] Store immutable map geometry, placements, and type metadata once per
  world identity. Per-tick episodes reference static data and retain dynamic
  state rather than copying the entire map.
  - [x] World inventories and their BVH spatial indexes are distinct
    content-addressed artifacts. Canonical inventory bytes can now be decoded
    and validated without reopening the extracted stage resources; alternate
    JSON spellings, inconsistent raw collision codes, invalid source/prism
    ordering, nonfinite geometry, and non-reproducible KCL/SCLS trigger joins
    fail closed. `world spatial-index` and every point/AABB/ray query accept the
    validated inventory artifact directly. A live F_SP103 rebuild retained
    10,794 prisms and 40 load triggers in inventory
    `370675af...01e7f`; its independently stored spatial index is
    `2ad975ee...e73834`, and an artifact-only room-1 query reproduced prism
    2187 as the nearest surface.
  - [x] Bind each derived local-geometry view to its source shard and exact
    inventory/spatial-index digests, then derive per-state probes by reference.
    A live 125-step Ordon shard produced 250 fully covered pre/post observations
    and 8,000 bounded probes in view `8e46dfc0...c3c7013`, bound to native shard
    `a849e3b9...1b014` and F_SP103 inventory `370675af...01e7f`.
  - [x] Bind the actor type/profile catalog and complete required static-world
    set directly to episode/shard identity, including multi-stage trajectories.
    Native shard v2 separates the authenticated game-data SHA-256, immutable
    card fixture, pointer-free actor profile-table identity, and canonical
    multi-stage world-context SHA-256. `world context` sorts and seals every
    required inventory and derived spatial index against the exact disc bytes.
    A live mixed-outcome Ordon shard bound F_SP103 and F_SP104 context
    `194cbb4b...e3b71`, ISO `490ef919...3c814`, card fixture
    `4a704aa8...341996`, and profile catalog `d1920358...831e4`; Rust accepted
    all 256 transitions in shard `18fcf868...7063d`. Geometry view v2 joined all
    512 observations in view `22fcaad0...35a5c` and rejected the same shard
    when F_SP104 was omitted. The native executable also exported all 792
    pointer-free profile slots (759 actors) as a canonical 197,018-byte catalog;
    the independent Rust decoder recomputed the same semantic identity and
    installed content blob `56bf2ec6...23edf`.
- [x] Remove arbitrary learner-facing actor truncation. Learning-observation v4
  requires the complete process actor set and rejects inconsistent counts,
  truncation markers, or a non-complete selection rule. The separate controller
  tactic view remains explicitly bounded for its hot-path contract.
- [x] Enforce decision-phase correctness: model input may contain only state
  realized before its chosen action. Add explicit tests against one-frame and
  terminal-label leakage.
- [x] Buffer episodes in memory and write compact content-addressed binary
  shards, not one file per attempt or tick. Bind every shard to build, game
  data, checkpoint, observation schema, action schema, objective and fidelity.
  Each candidate is buffered as one canonical episode, independently zstd
  compressed and checksummed inside one shard; the Rust boundary authenticates
  the complete shard by SHA-256. Shard v2 additionally fails closed unless game
  data, memory-card fixture, actor-profile catalog and world context are distinct
  declared identities. The live two-candidate shard compressed 3,531,036 bytes
  to 51,761 bytes without creating per-tick or per-attempt files.
- [x] Bind every process-boot dependency, including the isolated memory card,
  save data and relevant configuration, into the boot/checkpoint identity.
  Materializing the same declared fixture in a fresh automation root must
  reproduce the same frame-440 state without borrowing durable state from a
  previous run. The canonical card fixture is copied into a fresh writable
  root for every process-boot run and identified as
  `card-fixture:xxh3-128:4a704aa81ab84bbda55e25a04b341996`. Two cold
  568-boundary replays produced the identical semantic-state series
  `635ded69...3ae0`, reached Link control at frame 439 and the Ordon Springs
  load at frame 567 with identical v6 boundary fingerprints, and left the
  source GCI byte-for-byte unchanged.
- [x] Add corpus inspection for missing channels, masks, NaNs, constants,
  discontinuities, set sizes, action coverage, outcome balance, duplicate
  trajectories, and possible identity leakage.
- [x] Prove observation on/off parity and raw-PAD/state-hash equivalence with
  cold playback. `huntctl trace observation-parity` now binds a complete
  all-channel ordinary trace, a trace from the real one-candidate native v4
  learning batch, and that batch's authenticated episode shard into one sealed,
  fail-closed report. Gameplay trace retains the exact `PADRead` result before
  JUT's non-idempotent clamp, and suffix batches may emit this read-only witness;
  the terminal batch tick is retained exactly once. Two fresh process/card/cache
  roots replayed the same 565-frame tape with capture off and on. Their complete
  v5 traces were byte-identical (`075bef84...de16`), all 565 exact multi-port
  raw-PAD samples matched (`b41fadc5...8d26`), all 565 gameplay-state projections
  matched (`1cc15f95...11d9`), and the 125 episode PAD samples matched the trace
  suffix (`1bf49d0b...24c5`). The sealed report is
  `d437cce3...9b685`; any disagreement is emitted as a named first divergence
  and fails the command.

**Gate 1:** one native Ordon batch produces complete replayable trajectories
for every success and failure, can be ingested directly by the learner, has no
silent truncation or future leakage, and preserves deterministic cold playback.

## 2. Make experience collection fast enough to learn

- [ ] Keep the game process, disc resources, and source checkpoint alive across
  many batches. Training must not relaunch or replay the boot prefix per
  candidate.
- [ ] Support validated intermediate checkpoints along recorded trajectories so
  short-horizon experiments do not replay unrelated earlier decisions.
- [ ] Profile checkpoint restore, simulation, observation capture, policy
  inference, corpus encoding, GPU work and CPU draw traversal independently.
- [ ] Implement a true no-present render sink: no visible window, presentation,
  shader compilation, or GPU submission during farming while retaining any
  CPU-side work still required for equivalent gameplay.
- [ ] Attempt to skip CPU draw traversal only after audited A/B/A runs prove
  identical future gameplay across representative movement, actor, event and
  loading states. A mismatch blocks the optimization.
- [ ] Run fixed logical ticks uncapped; never alter the simulated framerate to
  increase throughput.
- [ ] Scale persistent workers only after measuring the single-worker loop.
  Choose process count with checkpoint memory bandwidth and crash isolation in
  mind rather than an arbitrary client limit.
- [ ] Publish useful transitions/second, episode throughput, restore cost,
  observation cost, inference cost, corpus bytes/transition and CPU/GPU share.

**Gate 2:** the trajectory-producing learner loop is materially faster than
cold prefix replay, and every approved fast mode produces the same realized
PAD and gameplay sequence as ordinary playback.

## 3. Give learners general state and precise control

- [ ] Derive declarative learner views from the canonical episode envelope.
  Models may request or ablate channel families without recollecting episodes.
- [ ] Represent actors, contacts and nearby geometry as masked sets or graphs,
  not fixed actor slots whose iteration order becomes accidental meaning.
- [ ] Provide both absolute context and Link-, camera-, surface-, actor- and
  goal-relative features. Generic physics relationships are observations, not
  route rewards.
- [ ] Always retain exact raw PAD as the ground-truth action.
- [ ] Replace the 16-heading/full-magnitude learned catalog with a factorized
  hybrid action surface covering stick direction and magnitude, sub-stick,
  button edges/holds and duration at frame-precise resolution.
- [ ] Let a policy invoke bounded stateful tactics through the same native
  episode boundary. Tactics may query read-only world state, but their exact
  consumed PAD and every intermediate transition must be recorded.
- [ ] Begin with generic options already justified by control needs: maintain a
  relative heading/offset, seek a coordinate or portable actor identity,
  compose a short curve, control camera while moving, and synchronize a button
  edge with an observed action phase. Do not encode Ordon coordinates or the
  incumbent's corner frames.
- [ ] Support short observation history or recurrent state for action phases
  and hidden timers that a single frame cannot make Markov.

**Gate 3:** the same model-facing contracts execute raw input, precise
continuous adjustments and stateful tactics; every execution exports an
identical standalone tape, and the observation encoder handles different actor
and geometry set sizes without schema changes.

## 4. Build the first genuine learning loop

- [ ] Maintain a replay corpus containing demonstrations, successful and failed
  policy rollouts, randomized coverage and alternate terminal states. Preserve
  checkpoint, episode, branch and policy-generation lineage.
- [ ] Learn a goal-conditioned estimate of reachability and time-to-go from
  `state + goal + remaining tick budget`. Do not use distance to the Ordon exit
  edge or distance along the incumbent as the learned objective.
- [ ] Propagate the real terminal outcome backward through complete
  trajectories. Use n-step returns, replay, target isolation and uncertainty
  appropriate to the chosen Q/value method.
- [ ] Use reverse curriculum from actual successful states: learn the final
  viable action basin, then move validated checkpoints backward. A state is
  useful when an actual continuation reaches the predicate, not when it falls
  inside a hand-authored floating-point waypoint epsilon.
- [ ] Add hindsight goals so failures teach achieved transitions and local
  dynamics even when they miss the main predicate.
- [ ] Use a hybrid proposal policy: learned value/reachability ranks
  checkpoint-backed short-horizon exploration, while continuous trajectory
  optimization handles analog parameters and discrete search handles button
  edges. Simulation remains the outcome authority.
- [ ] Compare at least:
  - the existing endpoint/local mutation baseline;
  - a small fitted or Double-Q learner using the new corpus;
  - a learner/search combination capable of the hybrid continuous action
    surface.
- [ ] Add held-out checkpoint evaluation and negative controls: shuffled
  outcomes, action-only input, removed collision/geometry, removed actors and
  removed history. Reject models that memorize frame, RNG, checkpoint or tape
  identity instead of predicting held-out outcomes.
- [ ] Track terminal success, time-to-go error, calibration, critic disagreement,
  effective state/action coverage and success by checkpoint. Do not promote on
  training loss.

**Gate 4:** under equal simulated-tick budgets, learned state-conditioned
proposals produce more held-out successes or faster valid routes than the
procedural mutation baseline, and the advantage disappears under appropriate
negative controls rather than surviving through leakage.

## 5. Prove optimization and discovery separately on Ordon

### 5A. Demonstration-seeded optimization

- [ ] Ingest the 125-tick tape as one successful demonstration, not as a reward
  function or mandatory corridor.
- [ ] Train from the first-Link-control source with terminal success and tick
  cost as authority. Permit the policy to leave the demonstrated trajectory.
- [ ] Use the learned critic/curriculum to find and exhaust at least one valid
  sub-125 route.
- [ ] Export and cold-replay the winner five times with identical per-tick
  state, first-hit tick, terminal predicate and terminal fingerprint.

**Gate 5A:** deterministically beat 125 ticks, and equal-budget ablation shows
that learned state/action relationships—not a new Ordon-specific mutation
family—produced the improvement.

### 5B. Goal-only discovery

- [ ] Start from the same checkpoint and terminal predicate without supplying
  the demonstration tape, incumbent-relative mutations, path coordinates or
  route-progress features.
- [ ] Allow generic world observations, raw actions, tactics, intrinsic
  exploration and the shared mechanics corpus.
- [ ] Preserve diverse behavior rather than only the state with smallest
  Euclidean or exit-plane distance.
- [ ] Produce a successful route and cold-replay its realized tape.

**Gate 5B:** the system discovers a deterministic route to Ordon Springs from
the goal and world state alone. Its speed may initially trail the optimized
lane; discovery, not parity with the 125-tick demonstration, is the gate.

### 5C. Thin Skybook-supported setup

- [x] Boot directly into Telma's Bar with declared post-twilight game flags
  that place both Telma and Louise in the bar. Record the complete boot
  identity so the fixture is reproducible and all gameplay after tick zero
  remains input-only.
- [ ] Acquire text displacement by talking to Louise while entering Telma's
  dialogue trigger, causing Louise's meow cutscene to interrupt Telma's
  dialogue.
- [ ] Observe the fixed shared text-progression bits needed to distinguish the
  displaced state from an ordinary interrupted event. These are read-only
  learner observations and semantic proof, never gameplay writes.
- [ ] Confirm the consequence by speaking to Telma afterward and advancing one
  dialogue box at a time through text that normally would have progressed
  during the interrupted cutscene.
- [ ] Retain normal control cases—Telma alone, Louise alone, and failed overlap
  timing—so the learner can distinguish the successful event ownership and
  text-state transition from nearby failures.
- [ ] Export and cold-replay the exact raw PAD sequence with identical actor,
  event, text-progression, and terminal evidence.

**Gate 5C:** from the declared post-twilight Telma's Bar fixture, the system
repeatedly acquires text displacement and proves the one-box-at-a-time Telma
behavior without a gameplay write or a human-specified successful input frame.

## 6. Prepare for narrow-basin glitch discovery

Only after gates 1-5 work end to end:

- [ ] Add a quality-diversity archive over generic relational state: spatial
  region, contact/surface relationships, action phase, velocity, actor/item
  relationships, event/inventory changes and novel displacement.
- [ ] Add intrinsic exploration signals based on new state/contact/event
  combinations and learned-dynamics disagreement. They prioritize experiments
  but never prove a glitch.
- [ ] Mine recurring successful action sequences as candidate reusable options
  while retaining raw-PAD refinement.
- [ ] Require an input-only deterministic proof for every claimed outcome.

**Gate 6:** the system can retain and revisit rare physically interesting
states, compose precise setup and action timing, and prove one selected
narrow-basin outcome without a human supplying its successful frame sequence.

## Explicitly not current work

- Enumerating every stage, room, layer or spawn before the learning loop works.
- Triaging or reproducing the entire Skybook catalog.
- A general visualization workbench.
- Route-specific waypoint rewards or hard-coded wall-follow instructions
  presented as learning.
- Dumping raw process memory or duplicating the entire static map every tick.
- Scaling workers or model size to compensate for missing trajectories,
  identical failure rewards, coarse actions or leakage.
