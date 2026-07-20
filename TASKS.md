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
- Learning-observation v5 and milestone language 1.8 retain and query the exact
  256-byte temporary-event register bank through the authenticated read-only
  observation boundary. A Telma/Louise boot fixture was used to verify actor,
  event-byte and cold-boot observation plumbing. That is coverage evidence only;
  the fixture is not an active glitch target, successful setup, demonstration,
  or justification for specializing the learner around that interaction.
- Learning-observation v9 adds a typed, read-only player-resource component:
  health and capacities, currencies, time, ammunition, dungeon state,
  inventory/equipment selection, bottle and bomb quantities, and stable
  acquisition/collection bitsets. The checked native-writer/Rust-reader
  fixture proves exact values and rejects presence disagreement. A neutral
  16-entry D_MN01 survey emitted actor-catalog v5 and observation v9 for every
  case, with the component present in all 16 and zero coverage rejections
  (`1a16c1c4...d9bb274`). This is generic state collection, not a route,
  reward, tactic, or gameplay write.
- Learning-observation v10 resolves eleven Link-rooted semantic relationship
  roles—target, ride, held/grabbed actors, retained item actors and attention
  hint/catch/look targets—to the stable generations in the complete actor set.
  No pointer is serialized. Native and Rust reject unjoinable identities,
  noncanonical absence and player/type status disagreement. A neutral 16-entry
  D_MN01 survey emitted actor-catalog v6 with the component available in every
  case and zero coverage rejections (`aa36f22e...ff7c281`); all roles were
  correctly empty in that idle slice, so generic action probes are still
  required to measure role variation.
- Learning-observation v11 retains Link's already-realized background-collision
  solver state without invoking collision code: named solver flags, cached line
  endpoints, wall-cylinder geometry, ground/roof/water offsets, water mode and
  all three wall-circle configurations. The only game-header change is a
  const, automation-gated friend aperture; capture performs reads only. Native
  and Rust reject unknown flags, noncanonical absence, nonfinite geometry and
  player/type status disagreement. A neutral 16-entry D_MN01 survey emitted
  actor-catalog v7 with the component present in all cases and zero coverage
  rejections (`4e3c8311...207df1ee`). All 16 idle samples shared solver flags
  `0x00000008`, table size zero, water mode one and offsets 60/0/1000, so this
  is capture-path evidence, not evidence of useful transition diversity.
- Native actor-view v5 now preserves those roles as typed edges to the complete
  actor set, and actor-feature view v2 exposes an independently selectable
  player-relationship family as per-actor role flags. Present-but-empty is
  distinct from absent, unavailable and not-sampled; legacy shards therefore
  remain masked instead of fabricating negative examples. The v10 fixture
  proves the target edge joins runtime generation 7, legacy v6/v7 fixtures
  prove historical missingness, and canonical validation rejects detached or
  multiply-selected roles. This is general learner input, not an authored
  interaction sequence, benchmark tactic or shaped reward.
- `huntctl learn inspect-native` now audits authenticated native shards before
  learner ingestion: channel/mask presence and flag-bit variation, constant
  core channels, actor/RNG/collision-set sizes, exact PAD coverage, outcome
  balance, exact payload duplicates, same-source/same-input determinism
  conflicts, and outcome-separating identity candidates. The live Ordon v4
  shard reports 47-48 actors, 28 actor types, 60 exact consumed PAD states, all
  structured mechanics and flag channels present, no truncation or determinism
  conflict, and the expected warning that its two retained candidates are both
  failures.
- Native suffix execution now evaluates the exact selected authored milestone,
  rather than silently substituting a built-in Ordon exit. A live frame-440
  batch bound program `b8cbfafa...0c169` and definition
  `631b025f...11e57` to objective identity `d0d98dc2...d9c11`, retained a
  known 127-tick `ordon_spring_load_committed` success and a 128-tick neutral
  failure, and finalized shard `39e3171b...8e35`. `huntctl learn
  inspect-native` accepted all 256 transitions and 512 observations with
  complete 47-48 actor sets, two RNG streams, six collision surfaces, exact
  chosen/consumed PAD agreement, both terminal observations, and zero
  non-finite values, discontinuities, truncation, duplicate trajectories,
  determinism conflicts, or warnings. The independent host identity function
  reproduces the native XXH3-128 domain separation and rejects a changed or
  noncanonical predicate digest. The promoted full process-boot tape had
  already reached the same load at frame 567 in two fresh cold replays without
  a controller, model, or checkpoint, producing the identical v6 boundary
  fingerprint `545c3127...ca155`.

## 0. Audit whether a learner can see enough of the game

This comes before selecting another glitch to reproduce. The purpose is to
collect and validate information that a learner could use, not for a human or
scripted controller to perform the documented technique.

**Research boundary:** a catalog entry may contribute observation fields,
action capabilities, temporal requirements, or a read-only success oracle. It
must not become an authored setup, attempted reproduction, route, tactic,
checkpoint, reward corridor, or frame sequence. Researchers stop at specifying
what information and authority a learner would need; discovering how to use
them is learner work.

### 0A. Survey every bootable world entry

- [x] Generate a canonical catalog of stage, room, layer and spawn candidates
  from extracted game data and known loader metadata. `world boot-catalog`
  retains retail `PLYR` provenance, loader provenance, source inventory
  identity and unreadable stages rather than depending on whichever workbench
  cache files happen to exist. The real GZ2E01 pass produced 79 stages and
  9,084 candidate boots: 78 complete retail inventories and one explicitly
  unreadable inventory (`D_MN09`) instead of silently dropping it. The ignored
  retail-data test reproduces `build/stage-survey/boot-catalog.json`; its cases
  remain unverified until the survey below classifies their runtime outcomes.
- [x] Build a resumable native survey that boots every catalogued entry, waits
  for a semantic ready boundary, observes a bounded interval and records boot
  success, timeout, crash, invalid combination, conditional setup or unmet
  prerequisite rather than retrying forever. Do not author a tape per map.
  `huntctl survey` binds the canonical catalog, executable, disc, empty-card
  fixture, observer schema, fixed-step settings and policy into a resumable
  ledger; each case gets bounded isolated attempts and a generic neutral probe.
  A live observer-enabled run classified `D_MN01/room/0/point/0/layer/-1` as
  ready after 30 post-readiness ticks, retained all 75 observed actors without
  truncation, and sealed the exact trace, actor snapshot and state sequence.
  Bounded parallel execution persists each completion independently; an
  eight-client smoke batch classified the first 16 entries as ready with
  64-145 of 64-145 actors retained per entry and no truncation.
- [ ] For every successful entry, prove that the learner observation contains
  the complete active actor population. Inventory all actor profiles and all
  generally available enemy metadata; enemies are sparse enough that this data
  should always be retained rather than selected by proximity.
  - [x] Aggregate each ready case's authenticated end-of-probe actor snapshot
    into a sealed stage/profile report, rejecting missing, truncated,
    count-mismatched and duplicate-process artifacts explicitly. The 16-case
    D_MN01 smoke report verified every snapshot and found 58 profiles across
    64-145 active actors per entry (`d23a7f78...1ffa`). This inventories the
    expected population; it does not pretend the bounded trace actor selection
    is a complete learner observation.
  - [x] Retain the exact complete actor vector supplied to native learning
    observation v8 at the generic probe's terminal boundary and compare it
    against an independent actor-catalog walk at that same tick. Actor-catalog
    v4 records both read-only populations; the sealed coverage report rejects
    missingness, truncation, duplicate generations, count drift, ordering
    drift, or any shared identity/state-field mismatch. A fresh eight-client,
    16-entry D_MN01 batch verified 64-145 learner actors per case, all 16 cases
    and 58 profiles with zero rejections (`061ed9a4...cb0a`). This proves the
    capture path on the surveyed slice; the parent item remains open until all
    successful catalog entries have equivalent evidence.
- [ ] Audit universal actor information: stable runtime identity and spawn
  generation, authored placement identity, transform, velocity, scale,
  collision/body information, action/procedure and animation state,
  health/status, room/layer, target, ownership, parent/child relationships and
  lifecycle events.
  - [x] Carry universal base-process and base-actor state through the same
    complete read-only population: process/actor type, condition and lifecycle
    phase, room history, pause/cull/demo/carry state, heap/model/joint-collision
    presence, old/current/home/eye transforms, scale, gravity and fall speed.
    Learning observation v7 (retained by current v10) is decoded into actor view
    v4 with explicit
    `None` for pre-v7 recordings rather than fabricated zeroes. Actor-catalog
    v4 independently captures and bit-compares the same fields. A fresh
    eight-client D_MN01 survey verified all 1,432 actors across 16/16 entries,
    58 profiles and 64-145 actors per entry with zero rejected cases in sealed
    report `95b05ccb...4ee6`. Procedure, animation and concrete collision-body
    state still require typed extensions, so the parent audit remains open.
  - [x] Capture Link-rooted relationships as named, optional, pointer-free actor
    identities: targeted, ride, held, grabbed, thrown-boomerang, copy-rod,
    hookshot-roof-wait, chain-grab, and attention hint/catch/look actors.
    Observation v10 requires every present role to join the complete actor
    population at the same boundary. Actor-catalog v6 independently exposes
    the roles, and a neutral 16-entry D_MN01 batch verified component presence
    and actor invariants in all 16/16 cases (`aa36f22e...ff7c281`). The parent
    remains open for non-player ownership/attachment and typed actor state.
  - [x] Carry those relationships through the offline learner boundary instead
    of dropping them after collection. Actor-view v5 stores canonical typed
    edges into the complete actor set; actor-feature view v2 turns each named
    role into a selectable, explicitly masked per-actor flag. V10, legacy and
    tamper tests prove exact joins, historical missingness and fail-closed
    model materialization without adding any route- or glitch-specific logic.
- [ ] Audit environment information: static and moving collision, nearby
  terrain triangles, contact normals and correction, materials, ceilings,
  floors, walls, triggers, exits, switches, event state and inactive authored
  placements.
  - [x] Retain the complete dynamic collision set processed by the immediately
    preceding collision pass without calling collision code: registration
    identity, owning actor, attack/target/correction enable and hit state, hit
    partner owners, raw type/source/result masks, attack power, weight, damage,
    correction vector, shape parameters and realized AABB. Learning observation
    v8 (retained unchanged by v9) and the cross-language fixture fail closed on
    noncanonical ordering,
    presence/payload disagreement and nonfinite geometry. A generic 16-entry
    D_MN01 survey retained 1,031 owner-joined bodies (0-91 per entry), comprising
    500 spheres, 529 cylinders and two explicitly unknown shapes, with no
    truncation, crash or rejected case; its actor-parity report is
    `061ed9a4...cb0a`. This exposes interaction geometry but supplies no
    interaction target, tactic, route or reward.
  - [x] Retain the current Link background-collision solver configuration and
    realized work geometry through a typed read-only component. Observation
    v11 records the known mode/contact flags, cached line, wall cylinder,
    offsets, water mode and all three wall circles; ordinary episode ordering
    supplies the raw per-tick sequence from which history can be derived. A
    neutral 16-entry D_MN01 batch verified component presence and actor
    invariants in all 16/16 cases (`4e3c8311...207df1ee`). Generic movement and
    contact probes plus a bounded-history learner view remain open, so the
    parent environment audit is not complete.
- [ ] Audit player, camera, inventory, timers, RNG, loading and other global
  channels across maps and forms. Record explicit missingness and phase rather
  than treating zero as unavailable.
  - [x] Capture player resources and inventory as a typed semantic component,
    rather than a raw save-memory dump: health/capacities, currencies, time,
    ammunition, dungeon possessions, inventory/equipment selections, bottle
    and bomb quantities, and acquisition/collection bitsets. Observation v9,
    corpus-inspection v3 and actor-catalog v5 preserve explicit channel status,
    canonicalize unavailable payloads, and reject disagreement with player
    presence. A neutral eight-client D_MN01 batch verified all 16/16 entries
    with zero rejected cases in sealed report `1a16c1c4...d9bb274`. The parent
    remains open for timers, loading state and cross-map/form variation.
- [ ] Run short generic observation probes where safe—idle, movement, camera,
  targeting, contact, basic actions, actor activation and loading—and use them
  only to expose changing fields. They are not route or glitch attempts.
- [ ] Visually inspect a stratified sample and reconcile visible actors,
  collision, triggers and state transitions against captured data.
- [ ] Publish a machine-readable coverage matrix by stage and actor profile:
  present, varying, absent, ambiguous, truncated, or requiring a typed
  extension. The survey must distinguish "not in this scene" from "not seen by
  the observer."

### 0B. Use Skybook as an observation/action requirements corpus

- [x] Index the Skybook catalog by underlying mechanism: precision movement and
  collision, actor displacement/interaction, targeting/camera, item and
  animation concurrency, event/trigger/loading state, RNG/timers, and actor
  lifecycle/memory behavior. The revision-bound
  `benchmarks/skybook/requirements-index.json` covers all 483 manifest pages,
  is derived only from canonical tags, titles, and descriptions, and has
  content digest `c610657b...9f07e`. Its validator recomputes every
  classification and rejects stale, reordered, detached, or edited entries.
  Location/platform/region tags are explicitly excluded as mechanism evidence,
  and page bodies are structurally unavailable to the classifier so published
  input sequences cannot leak into this artifact.
- [x] Deep-read a representative spread within each mechanism, stopping once
  the learner-facing requirements are identified. Do not reproduce the
  glitches, create setup tapes, or encode their published steps as tactics.
  The revision-bound review covers 20 pages across collision, actor
  interaction, targeting/items, events/loading, timers and lifecycle/capacity;
  only its controlled capability findings appear in
  `docs/glitch-hunting/learner-readiness-audit.md`.
- [x] For each studied glitch, ask only what a learner would require:
  - which ordinary controller actions must be expressible;
  - which pre-action state could contain useful signal;
  - which temporal history, actor relationships, collision relationships or
    hidden counters may matter;
  - what minimal read-only outcome predicate or generic invariant violation
    could prove success without encoding the technique;
  - whether the result could ultimately be proven by an exact cold-replayed
    input tape.
- [x] Classify each case as sufficiently observable/action-expressible, missing
  observations, missing action authority, missing a proof oracle, or outside
  the current deterministic model. This is a learner-readiness classification,
  not a claim that we know how to trigger it.
- [ ] Turn discovered gaps into stable universal channels or explicitly typed
  optional extensions. A map- or boss-specific extension must remain masked
  elsewhere and must not reorder or invalidate the cross-map schema.
- [x] Produce a learner-readiness matrix linking mechanism requirements to
  captured channels, actions and proof oracles. Do not select a specific
  Skybook glitch as an optimization target until this audit is reviewed. The
  current matrix intentionally classifies all 20 representative cases as
  missing observations: exact PAD authority exists, while the missing typed
  state/history prevents a defensible claim of learner readiness.

**Gate 0:** we can quantify what the learner receives at every bootable world
entry and explain, across representative Skybook mechanisms, whether the
observation/action boundary contains enough information and authority for a
learner to attempt discovery. The gate produces coverage evidence, not a
human-authored successful setup.

## 1. Turn every attempt into learning experience

- [x] Extend the native checkpoint batch loop to retain, for every candidate
  tick: pre-action observation, chosen action and exact consumed PAD,
  post-action observation, predicate/event changes, terminal status, remaining
  tick budget, and deterministic state identity. The batch runner now consumes
  the selected authored post-simulation definition instead of silently
  replacing it with the Ordon exit goal; shard metadata domain-separates and
  binds the exact program and definition digests, and Rust can verify that
  identity before ingestion.
- [x] Capture failures and successes through the same episode contract. A
  failed attempt must no longer collapse to one terminal X/Z observation.
- [ ] Define a canonical, versioned observation envelope containing:
  Capture every cheap, typed, read-only channel that can be made semantically
  stable, with explicit validity and provenance. Do not feed the canonical
  envelope to a learner as one flat vector: derive selectable set/graph views,
  masks and temporal features from the retained evidence, and compare those
  representations without recollecting rollouts. Raw process memory, pointers,
  padding and host-only implementation state are not observation features.
  - [x] Link motion, facing, action/animation phase, timers, form and relevant
    status;
  - [x] typed player resources, inventory/equipment selections, ammunition,
    dungeon possessions and stable acquisition/collection bitsets, with
    explicit presence rather than fabricated zeroes;
  - [x] named Link-rooted target, ride, held/grabbed, retained-item and
    attention relationships, joined by stable generation to the complete actor
    set rather than exposing process pointers;
  - [x] recent controls and camera state;
  - [x] realized background-collision contacts, surface/polygon identity,
    backing metadata and resolved planes/normals;
  - [x] current background-collision solver modes and cached configuration,
    including line, wall-cylinder, offsets, water mode and wall circles;
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
  - [x] derive Link-, camera-, and structural parent-relative actor features
    without changing the canonical raw actor set. `learn actor-view` binds the
    exact native shard and pointer-free profile catalog, retains complete actor
    identity/profile joins, and derives explicit-missing absolute-to-relative
    transforms offline. The live mixed-outcome Ordon shard produced 24,335
    actor nodes across all 512 observations, with 512 valid camera frames and
    2,560 resolvable parent relations, in sealed view
    `dcb396bb...fae3a` / content blob `11aad6d5...6c52`;
  - [ ] expose available typed actor components and goal-relative actor
    features without adding gameplay writes or making optional components look
    universally present;
    - [x] Actor base-state component v1 carries the universal observation-v7
      fields through native shards into actor view v4 with explicit absence for
      v2-v6 shards. It remains a structured component rather than being
      unconditionally concatenated into every model input;
    - [x] Learning observation v6 copies active actor attention and
      event-participation scalars through the single gated read-only adapter.
      Pointer fields never cross the boundary; constructor-default/inactive
      storage becomes an explicit absent component. Rust decodes v2-v6, rejects
      noncanonical masks/payloads, and the sealed actor view derives Link- and
      camera-relative attention positions offline. The v6 cross-language
      fixture covers both present and absent components;
    - [x] Retain the selected compiled goal as an authenticated semantic graph,
      including every Boolean edge, ordered sequence root, typed field/query,
      actor selector, comparison, literal and spatial primitive. The graph
      reconstructs and recompiles to the exact definition digest, rejects
      cycles/shared/unreachable nodes, and exposes typed spatial anchors. The
      older 65-float hash/statistics vector is not considered a semantic goal
      representation. The live Ordon view embeds all five exact transition
      atoms and four Boolean nodes in graph `e68befb3...12cae`;
    - [x] Feed the semantic goal graph to model encoders instead of treating
      digest bytes and predicate counts as learnable goal meaning. Goal input
      v1 emits masked typed node/projection rows, explicit Boolean child edges
      and ordered sequence roots; identity digests remain provenance only.
      Goal-conditioned policy/value input v3 and fixed representation v3 carry
      that variable-length structure directly. Renaming an otherwise identical
      definition leaves its model features unchanged, while literal and
      topology changes alter them and tampering fails closed;
    - [x] derive spatial relations to typed anchors in the selected compiled
      goal when the goal actually exposes one, retaining explicit absence for
      nonspatial or ambiguous goals. Actor view v3 binds the exact graph,
      resolves static AABB/plane and portable placed-actor anchors per
      observation, emits explicit stage-mismatch/absent/ambiguous states, and
      derives Link- and actor-relative positions. The exact nonspatial Ordon
      goal correctly produced zero fabricated anchors across all 512
      observations in sealed view `890a7787...bd471` / blob
      `1f17770d...3c36a`;
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
  Authored objectives use a domain-separated identity over the exact compiled
  program and selected definition digests; native suffix execution clones and
  resets that authored tracker at the source checkpoint, while Rust can
  independently recompute and verify the binding.
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
  The same-process substrate now preserves raw checkpoint bytes separately from
  semantic gameplay identity and permits only explicitly declared host-ABI
  padding to be canonicalized. A frame-440 A/B/A probe matched a 100-tick
  future across two validated and ten trusted restores; twelve independent
  frame-500 probes matched 40-tick futures across 36 trusted restores. The
  latter stress lane exposed intermittent `dMeter_drawHIO_c` padding changes;
  PDB-resolved field boundaries now exclude exactly its eight top-level ABI
  gaps without changing or weakening raw restore integrity. This proves the
  restore primitive, not yet its integration into the persistent batch loop.
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
  - [x] Derive a sealed dynamic-collision graph from the complete native-v8
    collider population without recollecting gameplay. Collision view v1 keeps
    not-sampled, unavailable, absent and present distinct; joins collider
    owners and attack/target/correction hit partners to actor generations; and
    retains unresolved runtime identities instead of silently dropping edges.
    Shape geometry is available in absolute, Link-relative and owner-relative
    coordinates with exact status masks. V8 and legacy-v7 fixtures cover
    complete-set joins and explicit historical absence; malformed edges,
    status/mask disagreement, non-finite geometry and canonical tampering fail
    closed. A trainable graph encoder and held-out comparison remain open.
- [ ] Preserve the complete dynamic actor set until an object-centric encoder
  has seen it. Learn attention/gating over typed channel families and explicit
  validity masks; do not permanently discard distant or apparently irrelevant
  actors with a hand-authored nearest-K rule. Any bounded inference view must
  expose its overflow and win an equal-budget held-out comparison against the
  complete-set representation.
  - [x] Remove the bounded controller's 256-entry limit from the independent
    learner set path. Actor-set encoding v2 accepts the authenticated native
    wire limit, rejects any declared truncation, binds the exact actor-feature
    schema, and permits explicitly exploratory encoders before a fixed-slot
    failure has been proven; evidence qualification remains separate from
    construction and promotion. A 257-actor regression proves complete-set and
    permutation-invariant encoding beyond controller capacity, while schema
    mismatch and non-finite aggregation fail closed. Equal-budget learned
    comparisons remain open.
  - [x] Derive a sealed, declarative native actor-feature view without
    recollecting gameplay. Feature view v1 keeps categorical integers exact,
    continuous and binary channels separately masked, runtime/parent identity
    as structural provenance rather than learnable ordinal values, and every
    actor in each observation. Its selectable families cover identity,
    absolute motion, universal lifecycle/physics, Link/camera/parent relations,
    attention, event participation and goal-relative positions. V7 and legacy
    V6 cross-language fixtures prove exact base-state values versus explicit
    absence, distinct schemas for channel ablations, canonical round trips and
    tamper rejection. A trainable graph encoder and native held-out comparison
    are still required before completing the parent item.
  - [x] Add an actual trainable complete-set baseline instead of treating
    summary statistics as DeepSets. The deterministic encoder learns a shared
    per-actor transform, masked mean/max pooling and nonlinear prediction head;
    exact categorical training values use non-ordinal channels, optional
    values retain masks, runtime generations remain structural only, and the
    sealed native actor-feature view materializes directly without dropping
    actors. A 257-node regression exceeds controller capacity, seeded refits
    are byte-identical, and a content-disjoint synthetic overflow task compares
    the model to a fixed-slot MLP with identical rows and optimizer steps. Both
    learned model identities and metrics bind the comparison artifact. This
    proves the trainable path and failure-detection experiment, not a live-corpus
    promotion; the equal-budget native held-out comparison remains open.
- [ ] Keep immutable world geometry out of the per-tick flat vector. Supply
  bounded spatial queries, connectivity and semantic surface/trigger tokens by
  reference to the authenticated world artifact, with a graph path for models
  that need context around a bend. Retain the source query and coverage proof
  so a larger view can be derived without rerunning gameplay.
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

- [ ] Treat optimization and discovery as distinct evaluation regimes over one
  learner, not as different route-specific algorithms:
  - demonstration-seeded runs may use successful episodes to initialize replay,
    state coverage and reverse curricula, but may not use incumbent-relative
    coordinates, progress rewards or a mandatory trajectory corridor;
  - goal-only runs receive the source state, semantic terminal predicate and
    generic observations/actions, with no successful action sequence;
  - transfer runs hold out maps, checkpoints and mechanics combinations so a
    learner must reuse control/contact knowledge instead of starting a fresh
    route model or memorizing one stage.
- [ ] Maintain a replay corpus containing demonstrations, successful and failed
  policy rollouts, randomized coverage and alternate terminal states. Preserve
  checkpoint, episode, branch and policy-generation lineage.
- [ ] Pretrain and continually refresh the shared state encoder from every
  phase-correct transition, not only successful episodes. Compare bounded
  auxiliary objectives such as next-state/delta prediction, inverse action,
  contact/surface transition, actor lifecycle, action phase, event/loading and
  short-horizon reachability. These objectives teach representation; they must
  not replace the real predicate and tick cost as outcome authority.
- [ ] Measure learned feature selection rather than assuming that more inputs
  helped: report attention/gating stability, held-out prediction by channel,
  rare-event recall and controlled channel-family ablations. Reject a broad
  encoder that loses to the smaller view under equal data and simulator budget,
  while preserving the canonical raw evidence for a different encoder.
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
- [ ] Build reset distributions from states the learner actually reached,
  clustered by semantic/relational state rather than frame number or distance
  along a tape. Expand the curriculum frontier only when held-out rollouts can
  reconnect a newly explored state to a terminal success or another validated
  competence basin.
- [ ] Add hindsight goals so failures teach achieved transitions and local
  dynamics even when they miss the main predicate.
- [ ] Learn reusable goal-conditioned skills/options from experience: discover
  coherent action phases, parameterize them by relational targets and state,
  learn initiation/termination conditions, and always retain the exact raw-PAD
  realization. Options may be proposed by the learner or mined from recurring
  behavior; they must not embed route coordinates or documented glitch steps.
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

## 6. Prepare for narrow-basin glitch discovery

Only after gates 1-5 work end to end:

- [ ] Add a quality-diversity archive over generic relational state: spatial
  region, contact/surface relationships, action phase, velocity, actor/item
  relationships, event/inventory changes and novel displacement.
- [ ] Preserve precision basins rather than only terminal winners: archive
  states by fine relational offsets, velocity/contact phase, simultaneous
  interaction opportunities and critic/model uncertainty. Adapt resolution
  around empirically sensitive dimensions instead of globally hand-selecting
  a floating-point tolerance.
- [ ] Add intrinsic exploration signals based on new state/contact/event
  combinations and learned-dynamics disagreement. They prioritize experiments
  but never prove a glitch.
- [ ] Mine recurring successful action sequences as candidate reusable options
  while retaining raw-PAD refinement.
- [ ] Test compositional discovery: combine independently learned setup,
  locomotion, interaction and frame-synchronization options, then let
  short-horizon continuous/discrete refinement search their boundaries. The
  benchmark must include a success whose setup requires multiple simultaneous
  relational conditions so raw local mutation is an inadequate baseline.
- [ ] Require an input-only deterministic proof for every claimed outcome.

**Gate 6:** the system can retain and revisit rare physically interesting
states, compose precise setup and action timing, and prove one selected
narrow-basin outcome without a human supplying its successful frame sequence.

## Explicitly not current work

- Manually reproducing Skybook entries during the information-coverage audit.
- Turning documented glitch procedures into routes, tactics, checkpoints or
  shaping rewards for the learner.
- A general visualization workbench.
- Route-specific waypoint rewards or hard-coded wall-follow instructions
  presented as learning.
- Dumping raw process memory or duplicating the entire static map every tick.
- Scaling workers or model size to compensate for missing trajectories,
  identical failure rewards, coarse actions or leakage.
