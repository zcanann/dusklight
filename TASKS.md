# Learning framework execution plan

The objective is a deterministic, native-speed learning system that can discover
and optimize controller-input solutions which are meaningfully better than a
human editing a tape frame by frame.

This is an ordered execution plan, not a wishlist. The Ordon Springs 125-tick
route remains the first integration benchmark, but it does not define the
architecture. Skybook is a requirements corpus used to test whether the
framework exposes enough of the game to control difficult glitches; it is not a
checklist we promise to reproduce blindly.

## Non-negotiable invariants

- Identical input and identical initial state must produce identical per-tick
  gameplay state. Any disagreement is a framework bug to investigate, never a
  reason to search for a more timing-tolerant tape.
- Normal observation is read-only. Fork-owned observers may copy already
  realized game state behind `DUSK_ENABLE_AUTOMATION_OBSERVERS`; they may not
  change gameplay logic, object layout, initialization, control flow, caches,
  RNG, collision state, or outcomes.
- A learned controller may inspect state while searching, but every promoted
  result is the exact raw PAD sequence it consumed and must pass repeated,
  input-only cold playback.
- Simulation time is fixed logical ticks. Rendering, shader compilation, host
  I/O, process startup, and checkpoint restore time never enter a route score.
- Per-tick capture, policy execution, and input application stay native. Rust
  may schedule batches, manage corpora, train models, and promote artifacts; it
  is not a per-frame dependency.
- Checkpoints accelerate experiments but never establish correctness. They must
  reproduce the same future state sequence as their cold-boot prefix.
- Comprehensive capture and learner input are separate contracts. Preserve a
  rich canonical record, then derive versioned learner views from it so a newly
  useful feature does not require repeating every old experiment.
- Route-specific geometry, waypoint rewards, and incumbent-relative mutations
  are allowed only as declared baselines. They are not evidence that a learner
  discovered a solution.
- Methods are compared under equal simulated-tick budgets. Promotion is based
  on terminal predicates, deterministic replay, and held-out behavior rather
  than training loss or algorithm names.

## 0. Research the control surface before designing the learner

This phase is deliberately first. We do not know enough yet about which state
is universally available, which state varies by stage, or which missing facts
make representative glitches unlearnable.

### 0.1 Inventory and boot the complete stage universe

- [ ] Build an authoritative inventory from game data of every declared stage,
  room, layer, and spawn-point combination. Do not probe guessed numeric ranges
  and mistake crashes for a catalog.
- [ ] Record the fixture requirements that affect a valid boot: form, entrance,
  inventory, event/save flags, time, mounted state, and any stage-specific
  dependencies discovered during enumeration.
- [ ] Boot every catalogued spawn through the native harness. For each origin,
  retain readiness, crash/timeout, initial fingerprint, and a bounded neutral
  trace followed by a small standardized input probe.
- [ ] Repeat every boot sufficiently to detect nondeterminism. Compare observer
  on/off and headful/render-sink execution; quarantine the first divergent tick
  and component.
- [ ] Produce a compact review queue. Inspect at least one headful run for every
  distinct stage/room/layer scene configuration, plus every spawn whose actors,
  collision, triggers, readiness, or outcome differ from its siblings.
- [ ] Replace the current one-entry stage-smoke fixture with a generated,
  versioned coverage catalog and a human-readable coverage report. The report
  must distinguish valid, conditionally valid, invalid-by-game-data, crashing,
  timed-out, and not-yet-understood origins.

For every boot, audit these families rather than merely proving that Link
appeared:

- Link, mount, camera, controller history, action/procedure/animation, timers,
  velocity, form, and inventory-relevant state;
- every active gameplay actor, including enemies, NPCs, pickups, projectiles,
  movable objects, doors, switches, and actor-backed tags/triggers;
- actor identity, type/profile, set and room identity, parameters, parent/owner,
  transform, velocity, health, status, action/animation, collision volume,
  contacts, targeting, timers, and other deterministic fields justified by the
  observer audit;
- static collision and placement data, plus dynamic contact manifolds, surface
  identity/normals, clearance, scene exits, event regions, and non-actor
  triggers;
- event/cutscene state, flags, RNG state/counters, loading and transition state,
  UI ownership, and resource/heap facts that may affect gameplay;
- explicit absence, staleness, invalidity, and truncation for every channel.

The actor trace currently retains at most 16 non-player actors. That is useful
diagnostic scaffolding, not acceptable learner coverage. The research capture
must retain every active gameplay actor without arbitrary selection. Static
actor/type/placement metadata should be stored once; dynamic records should be
variable-length, deterministically ordered sets keyed by stable portable
identity. Native pointers must never become identities or learned features.

**Gate 0A:** every game-data-declared spawn has a classified boot result and
repeat evidence; every distinct scene configuration has been reviewed; no
successful run silently truncates an audited observation family.

### 0.2 Audit Skybook for controllability, not replication

The checked corpus currently contains 483 pages, including 452 categorized as
glitches. Its dominant tags already span movement, collision, cutscenes, warp,
memory behavior, storage, combat, actor corruption, crashes, RNG, and many
map-specific mechanisms. A route optimizer designed only around Link position
cannot credibly address that surface.

- [ ] Create a structured, revision-bound requirements inventory for every
  Skybook page. This is triage, not an implementation commitment.
- [ ] For each page, record the documented initial conditions, controller
  actions, timing precision, relevant actors and geometry, hidden state, RNG or
  platform dependencies, expected outcome, likely checkpoint boundary, and a
  semantic proof candidate.
- [ ] Classify every required fact as `captured`, `derivable`, `missing`,
  `unsafe-to-query`, or `unknown`. Classify every required action as raw-PAD
  expressible, expressible by an existing native tactic, missing a useful
  tactic, or dependent on an intervention that cannot prove a real glitch.
- [ ] Group pages by causal mechanism rather than by map or title. At minimum,
  cover movement/camera, static collision, actor displacement and combat,
  pickup/item synchronization, mounted movement, trigger/event/cutscene state,
  storage/flags/inventory, loading/warp/save state, RNG, resource/actor-slot
  pressure, memory corruption, crash, and renderer-visible consequences.
- [ ] Deep-read representative pages from every mechanism family, including
  several narrow, frame- and float-sensitive setups. Trace their required facts
  into actual game types and existing observer fields. Do not infer coverage
  merely because a generic `position` or `status` field exists.
- [ ] For each representative, boot its relevant scene when known and capture a
  short native trace. Confirm whether the referenced actors, surfaces, triggers,
  action phases, timers, and outcome evidence are actually present and stable.
- [ ] Record gaps as generic capabilities where possible. For example, prefer
  `contact manifold plus action phase` over a feature named for one Ordon wall.
  Preserve genuinely type-specific state as a versioned actor component rather
  than forcing unrelated maps into a fake universal scalar layout.

The resulting matrix must answer two separate questions:

1. Could a state-aware policy control the documented setup with the facts and
   actions currently available?
2. Could Dusklight prove the outcome without patching gameplay or relying only
   on a screenshot?

An `unknown` answer is valid research output. Pretending that a coarse actor
record is sufficient is not.

**Gate 0B:** each major Skybook mechanism has at least one source-backed
control/proof analysis and one relevant runtime inspection where a scene is
known. Every observed gap is tied to a concrete example, and no unselected page
has silently become a promise to reproduce it.

### 0.3 Freeze the first evidence-driven benchmark ladder

- [ ] Select a small ladder only after gates 0A and 0B:
  1. Ordon Link-control to `ordon_spring_load_committed`, testing route learning;
  2. a movement goal on a held-out map, testing transfer rather than memorized
     coordinates;
  3. one Skybook-supported, narrow collision/actor/timing setup, testing precise
     precondition acquisition and synchronized execution.
- [ ] For each benchmark, define the initial-state identity, terminal predicate,
  allowed observation families, action surface, simulated-tick budget, training
  budget, and cold-playback promotion gate before training begins.
- [ ] Retain hand-authored tapes, hard-coded tactics, random search, and the old
  Ordon endpoint search as explicit equal-budget baselines.

**Gate 0C:** the ladder covers route optimization, cross-map transfer, and a
thin success manifold without requiring the whole future framework at once.

## 1. Build a lossless native transition corpus

- [ ] Replace endpoint-only suffix results with a native per-decision record:
  pre-action observation, exact chosen action/PAD, post-action observation,
  events, terminal status, predicate progress, and state-sequence identity.
- [ ] Define a versioned canonical observation envelope with variable-length
  typed sets for actors, contacts, triggers, exits, and events. Missing fields
  require explicit masks; arbitrary fixed actor slots are forbidden.
- [ ] Store static stage geometry, placements, and immutable type metadata once
  by game-data identity. Per-tick records reference those objects and contain
  only dynamic state.
- [ ] Capture all active gameplay actors by default. If a measured throughput
  optimization later filters a learner view, the canonical episode must still
  disclose what was omitted and permit rebuilding a different view.
- [ ] Add deterministic history for facts that are not Markov in a single
  sample: recent PAD, contacts, action transitions, camera motion, spawn/despawn
  events, and predicate changes.
- [ ] Write compact, content-addressed episode shards from in-memory batches.
  Bind build, game data, scenario/checkpoint, observation schema, action schema,
  objective, seed, and fidelity. Do not write a file per attempt or per tick.
- [ ] Feed successful demonstrations, ordinary failures, near successes,
  randomized exploration, and learner rollouts into the same corpus contract.
  Preserve episode and branch boundaries.
- [ ] Add corpus inspection that reports feature availability, actor/set size
  distributions, truncation, constants, NaNs, leakage, action coverage, success
  balance, and state/next-state discontinuities.
- [ ] Run observation-off/on A/B/A parity for every newly admitted field. Any
  observer-induced divergence blocks that field and its dependent models.

**Gate 1:** an in-process Ordon batch yields a replayable trajectory for every
attempt rather than one terminal X/Z point; the same contract records audited
actors on multiple structurally different maps with no silent truncation; raw
PAD and state hashes agree with equivalent cold playback.

## 2. Make local experience cheap without changing simulation

- [ ] Keep native workers persistent across batches. Process launch, boot, and
  prefix replay must not occur per candidate.
- [ ] Support validated checkpoints at the source of a segment and at useful
  intermediate curriculum boundaries. Restore must include every state
  component required to reproduce the next validation window exactly.
- [ ] Profile capture, restore, simulation, observation, inference, and artifact
  encoding separately before optimizing any one of them.
- [ ] Implement and measure a no-present render sink first. Remove draw traversal
  only if observer-off/on and headful/renderless state sequences prove that the
  skipped work has no gameplay consequence on the audited stage suite.
- [ ] Keep crash-prone experiments in disposable worker processes while still
  batching many safe attempts per process. A crash must lose at most the active
  uncommitted batch and must retain its input/model/checkpoint identity.
- [ ] Choose worker count, CPU affinity, batch size, observation density, and
  checkpoint tier from measured candidate-ticks per second and deterministic
  restore cost, not an arbitrary client limit.

**Gate 2:** the same raw tapes produce identical gameplay sequences in the
approved execution modes, and local checkpointed training gains enough measured
throughput to justify its complexity over cold prefix replay.

## 3. Expose a learnable, cross-map state and action language

### Observation representation

- [ ] Keep a stable outer schema: player, recent control, camera, contacts,
  actors, triggers/exits, local geometry, global stage context, and goal.
  Contents may vary by map without changing the envelope.
- [ ] Encode actors, contacts, and nearby geometry as permutation-invariant sets
  or graphs. Include type and component masks so enemies, pots, NPCs, pickups,
  and stage mechanisms can expose different typed state safely.
- [ ] Separate a shared Link/camera/collision encoder from map geometry context,
  goal encoding, and small map- or mechanism-specific adapters. A new map may
  add an optional typed component; it may not redefine every existing feature.
- [ ] Preserve absolute world context while also deriving Link-, camera-,
  surface-, and target-relative features. This allows transfer of concepts such
  as clearance, approach angle, wall contact, and actor interception without
  prescribing a route.
- [ ] Permit privileged read-only state during training and critic evaluation
  when the audit justifies it. Record that dependency explicitly; the promoted
  tape remains raw input-only.
- [ ] Make observation views declarative and ablatable. Train/evaluate with
  channel families removed to prove which information contributes signal.

### Action representation

- [ ] Always retain exact raw PAD as the ground truth action.
- [ ] Provide a factorized native action surface for analog stick direction and
  magnitude, camera/sub-stick, button edges, holds, and durations. Do not force
  fine movement into a handful of full-magnitude compass headings.
- [ ] Expose bounded stateful tactics only where the research audit demonstrates
  recurring value: seek an actor or coordinate, maintain relative offset,
  follow/compose a curve, control camera while preserving movement, time rolls,
  and synchronize an action edge with an observed phase.
- [ ] Treat tactics as temporally extended policy options whose exact consumed
  PAD is recorded. They may improve exploration and credit assignment, but a
  learner must remain able to choose or refine raw per-tick input.
- [ ] Resolve targets through portable actor/placement/type identities and
  explicit selection rules, never pointers or accidental iteration order.

**Gate 3:** one model-facing schema batches variable actor and geometry sets
from multiple maps; actions cover both coarse traversal and frame-precise analog
adjustments; every policy execution materializes an identical standalone tape.

## 4. Establish that learning, rather than mutation volume, drives progress

- [ ] Define a goal-conditioned interface approximating value and policy from
  `state + goal + remaining tick budget`, not a separate hard-coded network
  input for every route.
- [ ] Use terminal success and tick cost as the authoritative task reward. Add
  hindsight goals, reverse curriculum, reachability estimates, or representation
  objectives to make sparse success learnable; do not encode the incumbent's
  waypoints as the answer.
- [ ] Bootstrap from human demonstrations when useful, but keep a source-only
  lane with no demonstration-relative distance, mutation position, or corridor
  features. A demonstration is one successful episode, not the reward function.
- [ ] Start local curricula near real success states, learn the viable basin of
  precise actions and preconditions, then move checkpoints backward. Preserve
  alternate successful states instead of assuming the fastest local state has
  the best continuation or RNG.
- [ ] Maintain replay mixtures across successes, hard failures, diverse states,
  maps, goals, and policy generations. Use priority and uncertainty without
  allowing a tiny set of terminal transitions to dominate value scale.
- [ ] Compare suitable learners under the same corpus and simulation budget.
  At minimum include a non-learning action-search baseline, the existing fitted
  or Double-Q path, and a method capable of the audited hybrid/continuous action
  space. Select algorithms from held-out terminal performance and sample
  efficiency, not from the label `DQN` or `DDQN`.
- [ ] Add negative controls: shuffled outcomes, removed geometry/actor channels,
  action-only prediction, and held-out checkpoints. A claimed signal that
  survives shuffled labels or fails held-out states is not useful learning.
- [ ] Track calibrated success probability, time-to-go error, critic ensemble
  disagreement, state/action coverage, and terminal success by checkpoint and
  map. Training loss alone is never a campaign result.

**Gate 4:** under equal simulated-tick budgets, the learner repeatedly produces
more held-out terminal successes or faster valid routes than random/local tape
mutation and the current procedural proposal families. Results must survive a
fresh process and exact cold replay.

## 5. Prove the architecture on the benchmark ladder

### 5.1 Ordon Springs

- [ ] Re-run the 125-tick segment with the new trajectory corpus. Do not use the
  hard-coded Ordon exit edge, incumbent-relative waypoint rewards, or fixed
  mutation windows as learner input.
- [ ] Verify that contacts, wall-relative geometry, camera, action/roll phase,
  and the full relevant actor set are available before deciding what the model
  should ignore.
- [ ] Train from the same source checkpoint with a declared demonstration lane
  and a demonstration-free lane. Report sample budget and learning curves for
  both.
- [ ] Export every improvement as raw PAD, repair downstream segments only when
  requested, and cold-prove any promoted tape repeatedly.

**Gate 5A:** deterministically beat 125 ticks, and ablation/equal-budget evidence
shows that state-conditioned learning—not an Ordon-specific proposal family—was
responsible for finding useful actions.

### 5.2 Held-out map transfer

- [ ] Hold one audited movement scene out of shared representation/dynamics
  training.
- [ ] Compare adaptation from the shared model against training the same model
  from scratch under an equal local sample budget.
- [ ] Record which generic channels transfer and which optional map components
  are newly required. Evolve the versioned schema rather than forking an
  unrelated format.

**Gate 5B:** the shared model reaches the held-out goal with a measured sample
efficiency advantage, or the failure produces a specific representation gap
supported by traces and ablation.

### 5.3 Thin Skybook-supported setup

- [ ] Select the representative only after the controllability audit. Prefer a
  case requiring a narrow combination of relative position/angle/velocity,
  action phase, and actor or pickup timing.
- [ ] Define separate semantic predicates for reaching the precondition region,
  executing the synchronization, and obtaining the actual outcome. These aid
  diagnosis and curriculum; only the real outcome authorizes promotion.
- [ ] Learn locally from validated checkpoints, expand the successful basin,
  move the curriculum backward, and finally produce an input-only tape from its
  declared boot origin.

**Gate 5C:** the framework finds and cold-replays the selected outcome without
gameplay writes, hard-coded outcome injection, or a human specifying the exact
successful frame sequence.

## 6. Only then open unknown-glitch discovery

Do not build a general novelty workbench before gates 0 through 5 establish that
the framework can observe, control, learn, and prove known bounded outcomes.
Afterward, use the same corpus and native loop to add generic invariant and
novelty objectives such as unexpected collision crossing, discontinuous motion,
impossible actor overlap, action-state anomalies, event/flag inconsistency,
resource exhaustion, or learned-dynamics disagreement. Keep novelty archives
separate from semantic proof, and require deterministic input-only replay before
calling any candidate a game glitch.

## Existing evidence to retain as baselines

- Trusted checkpoint restore and cold-prefix equivalence have already been
  demonstrated for the current Ordon source state.
- Native suffix batches have evaluated 18,867 candidates and 2,358,375 suffix
  ticks without beating the 125-tick incumbent.
- Those batches mostly retained terminal endpoints and used route-specific
  mutation/ranking families. They establish that the current search is
  insufficient; they are not a serious learning corpus.
- The current `movement-state/v2` and selected-actor channels are useful seeds,
  but their fixed scalar layout, coarse action catalog, and 16-actor trace cap
  do not satisfy this plan's research or corpus gates.
