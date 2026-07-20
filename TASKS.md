# Learning framework execution plan

The objective is to build a system that learns reusable game mechanics and can
find or improve deterministic controller sequences more effectively than a
human editing inputs frame by frame.

This is an ordered execution plan, not a product wishlist and not a promise to
blindly reproduce the Skybook catalog. Each phase exists to answer a concrete
question required by the next phase.

## Non-negotiable rules

- Identical initial state plus identical consumed pad states must produce
  identical per-tick game state. Any run-to-run mismatch is a framework defect,
  never a reason to search for a more "robust" tape.
- A promoted result is an exact input tape that succeeds repeatedly from a
  clean, supported starting condition. Checkpoints accelerate experiments but
  do not prove results.
- Normal observation and control are read-only with respect to gameplay state.
  Observation code may copy state and controller code may supply pad input; it
  must not patch positions, actors, RNG, collision, flags, or procedures.
- Experimental state writes, if ever used to test whether a setup is possible,
  must be explicitly compiled and labeled as interventions. They cannot produce
  proof artifacts or silently enter the learning corpus.
- Observation and debug rendering must not change simulation behavior or game
  timing. Headful, no-present, and renderless execution must consume identical
  inputs and produce identical gameplay hashes.
- Per-tick simulation, observation, policy evaluation, and input application
  remain native and in-process. External tooling may configure experiments,
  train between batches, and manage artifacts; it does not conduct per-frame
  IPC.
- Route-specific hints may be used only in an explicitly demonstration-assisted
  benchmark. They must not masquerade as general learning.

## Current evidence

The checkpoint and native suffix machinery has already established useful
foundations:

- trusted in-process restore with A/B/A determinism checks;
- exact pad application at the native input boundary;
- compact execution of many suffix candidates;
- source-boundary fingerprints, predicates, gameplay hashes, and exact tape
  export;
- a 125-tick Link-control-to-Ordon-Springs incumbent.

The failed Ordon campaign is also useful evidence. It simulated 18,867
candidates and 2,358,375 suffix ticks without beating the incumbent, but retained
mostly terminal results. It therefore performed a large candidate search rather
than creating the per-tick transition corpus needed for learning. The immediate
work is not another hand-authored mutation family.

## 0. Research the observable and controllable game surface

This phase is deliberately first. We should not freeze a learning schema based
on the first route we happened to optimize.

### 0.1 Audit every stage, room, and spawn

- [ ] Generate an authoritative catalog of bootable stage, room, spawn, and
  layer combinations from the extracted game data and known loader metadata.
  Invalid or context-dependent entries must be classified rather than retried
  forever.
- [ ] Build an automated survey runner that boots every catalogued combination,
  waits for a semantic ready condition, executes a short fixed observation
  probe, and records success, crash, timeout, or unmet prerequisite.
- [ ] Do not create a hand-authored tape for every spawn. The survey is a
  resumable, deterministic batch whose cases and results are content-addressed.
- [ ] At each successful spawn, inventory all active actor slots and verify that
  the observer captures every actor rather than a nearest-N sample.
- [ ] For every actor, inventory which universal fields are meaningful:
  identity/profile, spawn generation, placement identity, transform, scale,
  velocity/displacement, collision bounds and contacts, procedure/action,
  animation, health/status, room/layer, ownership, targeting, and lifecycle
  events.
- [ ] Always capture all generally available enemy metadata for every active
  enemy. Record missing or suspect fields by enemy profile instead of deciding
  in advance that an enemy is irrelevant.
- [ ] Inventory map-authored placements, triggers, exits, switches, collision
  attributes, moving background collision, and other state that may exist even
  when its actor is inactive or unloaded.
- [ ] Compare runtime collision queries with the static stage inventory. Verify
  that nearby ground, wall, ceiling, dynamic collision, seams, and material
  properties can be represented without relying on the renderer.
- [ ] Exercise bounded universal probes where safe: idle, movement, camera,
  targeting, basic action, collision contact, actor activation, room exit, and
  return. These probes exist to reveal changing fields, not to solve routes.
- [ ] Visually inspect a stratified sample of survey runs and reconcile visible
  actors, enemies, collision, triggers, and state changes with captured data.
- [ ] Produce a checked-in coverage report summarizing, per stage/profile, what
  is captured, absent, ambiguous, unstable, or dependent on special setup.

The survey must be restartable and must retain enough evidence to distinguish
"the map contains no such feature" from "our observer failed to see it."

### 0.2 Audit Skybook for learner controllability

- [ ] Derive a mechanism taxonomy from Skybook descriptions and tags. At
  minimum distinguish precise movement/collision, ceiling and floor behavior,
  actor displacement, enemy manipulation, targeting/camera, item and animation
  concurrency, trigger/loading behavior, RNG/timers, lifecycle/slot behavior,
  and memory/heap corruption.
- [ ] Select representative examples across those mechanisms. This is a
  requirements sample, not a checklist of glitches to reproduce.
- [ ] For each example, identify:
  - the exact externally controllable inputs;
  - the smallest defensible success predicate or novelty condition;
  - required temporal precision and useful checkpoint boundaries;
  - player, actor, collision, camera, event, inventory, RNG, and lifecycle state
    needed to distinguish progress from failure;
  - whether the required state is already observable without gameplay writes;
  - whether a successful exact tape could be independently cold-replayed.
- [ ] Classify each example as observable and controllable, observable but
  missing an action primitive, missing required observation, lacking a proof
  oracle, or outside the current deterministic model.
- [ ] Pay particular attention to interactions that defeat an endpoint-only
  representation: sub-frame-looking action conjunctions resolved on one tick,
  near-float-precision wall offsets, actor pushes, pickups during attacks,
  collision-side changes, state carried across loading, and actor-slot or heap
  effects.
- [ ] Identify generic invariant violations worth detecting even without a
  known route: crossing collision without an exit, entering an unreachable
  spatial cell, discontinuous displacement, incompatible action/event pairs,
  wrong-side contact, unexpected actor push, lifecycle inconsistency, and
  stage/position disagreement.
- [ ] Publish a controllability matrix linking each required signal to the
  stable schema, a proposed optional extension, or an explicit unresolved gap.

**Gate 0:** the stage survey is complete enough to quantify coverage, and the
Skybook study demonstrates that the proposed observation/action boundary could
control a representative range of difficult mechanics. Unknowns are explicit;
no route-specific field is promoted merely because it helps Ordon.

## 1. Establish a stable, extensible observation contract

### 1.1 Observation envelope

- [ ] Define a versioned observation envelope containing build identity,
  schema manifest, simulation tick, restore identity, task identity, pre-action
  state, consumed input, post-action state, events, and gameplay hash.
- [ ] Assign stable semantic field IDs. New fields append through versioned,
  masked extensions; they never reorder existing data or make older episodes
  unreadable.
- [ ] Record availability masks and validity separately from numeric values.
  Zero is a valid game value, not a substitute for "unknown."
- [ ] Capture every executed transition, including failed and aborted attempts.
  Terminal-only candidate summaries are insufficient training data.

### 1.2 Object-centric actor observations

- [ ] Emit a masked token for every actor slot on every tick. Preserve the
  engine slot/process identity and add a spawn generation so slot reuse does not
  alias two actors.
- [ ] Give every actor a universal feature block and semantic type/profile
  embedding. Give every enemy the complete generally available enemy block.
- [ ] Add typed optional extensions for profile families, bosses, projectiles,
  switches, or map-specific mechanisms only when the research audit finds a
  meaningful signal. Models must remain functional when an extension is absent.
- [ ] Represent parent/child, target, carrier/carried, collision, and ownership
  relationships explicitly so a learner can reason over interactions rather
  than infer them from arbitrary slot order.
- [ ] Prove that collecting the complete actor table does not allocate, mutate
  actor state, alter process order, or perturb gameplay hashes.

### 1.3 Geometry and world observations

- [ ] Store immutable stage geometry, placements, triggers, and exits once per
  map/build in the corpus rather than duplicating them every tick.
- [ ] Emit a bounded local set of relevant static and dynamic collision
  surfaces. Each token should include stable identity, vertices or static
  reference, closest point, signed distance, normal, classification, material,
  contact state, and collision correction.
- [ ] Determine the local surface budget empirically from the stage audit. Use
  stable identity plus admission/eviction hysteresis so slots do not flicker
  merely because two triangles exchange distance order.
- [ ] Include absolute coordinates for exact proof and egocentric/player- and
  camera-relative features for cross-map learning.
- [ ] Treat moving collision and actor-owned collision as relationships to an
  actor token, not anonymous terrain.

### 1.4 Temporal state

- [ ] Preserve exact per-tick histories in the corpus and expose a bounded
  history or recurrent state to policies.
- [ ] Include lifecycle and transition events such as spawn, deletion, damage,
  contact begin/end, procedure change, pickup, trigger activation, loading
  request, and loading completion.
- [ ] Verify phase correctness: every field must document whether it is sampled
  before input, after gameplay, after collision correction, or after scene
  transition processing.

**Gate 1:** repeated observed and unobserved executions have identical consumed
inputs and gameplay hashes; all active actors and enemies are present; schema
evolution is proven using mixed-version episodes; and representative audited
mechanics have sufficient observable state or a documented gap.

## 2. Turn executions into a durable learning corpus

- [ ] Define a compact binary, content-addressed episode format. JSON may be an
  inspection view, not the hot or archival representation.
- [ ] Key an episode by build, boot/scenario, source checkpoint, exact input,
  observation schema, objective, and resulting state sequence.
- [ ] Retain successes, failures, novel states, and materially different
  terminal states. Do not retain only the current fastest tape.
- [ ] Store static map data once and encode per-tick actor/terrain changes as
  compact columns or deltas without losing exact reconstruction.
- [ ] Support prioritized sampling without mutating the immutable underlying
  evidence.
- [ ] Split training and evaluation by whole episode, checkpoint, and map where
  appropriate. Adjacent frames from one run must not leak across a held-out
  boundary.
- [ ] Record provenance for demonstrations, learned policies, random
  exploration, scripted probes, interventions, and human recordings.
- [ ] Support hindsight relabeling against predicates that are true in retained
  states, while keeping the originally requested objective intact.
- [ ] Add corpus coverage reports for stage/room, spatial cells, actor profiles,
  action/procedure states, contacts, events, outcomes, and schema availability.

**Gate 2:** a native campaign can produce a reusable transition corpus without
per-frame IPC or files; a separate training process can reconstruct episodes,
sample them deterministically, and reproduce held-out metrics from a manifest.

## 3. Build the native experiment and action substrate

### 3.1 Persistent execution

- [ ] Replace process-per-attempt and prefix-per-attempt execution with
  persistent native experiment instances restored from trusted checkpoints.
- [ ] Support multiple isolated simulation instances per process if measurement
  shows that shared immutable assets and in-process scheduling improve useful
  transitions per second. Instance state must not leak across attempts.
- [ ] Keep a small out-of-process supervisor for crash detection and replacement,
  especially for eventual memory-corruption experiments. It is not the
  per-frame controller.
- [ ] Capture intermediate checkpoints at useful deterministic boundaries so
  training a late precision task does not replay minutes of irrelevant prefix.
- [ ] Measure snapshot capture, restore, observation, policy, simulation,
  rendering, and corpus costs separately before designing more elaborate
  snapshot machinery.

### 3.2 Rendering modes

- [ ] Add and benchmark headful, no-present/no-window, and no-draw execution.
- [ ] Refuse to use an accelerated mode for learning until equivalence tests show
  identical per-tick gameplay hashes against normal rendering across a
  representative stage sample.
- [ ] Keep simulation ticks independent of wall-clock speed, audio, shader
  compilation, presentation, and host I/O.

### 3.3 Learnable actions

- [ ] Expose the complete raw pad action: continuous stick components, button
  edges/holds, triggers, camera input, and duration.
- [ ] Use a factorized or hybrid action model so precise analog control does not
  require enumerating a vast Cartesian set of stick and button combinations.
- [ ] Retain exact single-tick actions for precision and allow native stateful
  options such as movement curves, camera-relative travel, target-relative
  movement, roll/attack sequences, and termination conditions.
- [ ] Options are conveniences the learner may select and parameterize, not
  mandatory route recipes. Every executed option must lower to the exact pad
  states actually consumed.
- [ ] Allow overlapping continuous control, camera control, and discrete button
  events with explicit composition rules and a small bounded layer count.
- [ ] Feed realized actions, not merely requested option parameters, back into
  the corpus.

**Gate 3:** a persistent batch can evaluate policies over rich observations,
restore safely, and export any episode as an ordinary deterministic tape.
Renderless execution provides a measured speedup without gameplay divergence.

## 4. Make the system learn rather than rank a mutation list

### 4.1 Shared representation and dynamics knowledge

- [ ] Train an object-centric world encoder over global state, the complete
  actor set, local terrain set, relationships, and temporal history.
- [ ] Pretrain useful predictive tasks across the corpus: next player state,
  actor motion/state, contact and collision correction, action transition,
  event occurrence, and short-horizon reachability.
- [ ] Share universal movement, camera, collision, action, and actor-interaction
  representations across maps. Use map/profile embeddings and optional adapters
  for genuinely specific data.
- [ ] Add new extension fields without forcing full retraining or making old
  episodes unusable. Test whether an extension helps through held-out ablation
  before treating it as signal.

### 4.2 Goal-conditioned value and policy learning

- [ ] Represent the requested outcome as a predicate/event program plus tick
  budget, not a route or required waypoint sequence.
- [ ] Start with a sample-efficient Double-Q or fitted-Q baseline with target
  models, replay, and uncertainty/disagreement estimates. Do not enable a model
  merely because training loss falls.
- [ ] Combine the critic with a hybrid policy suitable for continuous sticks and
  discrete button timing. Compare direct continuous policies, parameterized
  options, and local trajectory optimization under equal simulated-tick budgets.
- [ ] Train on full transitions from successful and failed episodes. Use
  prioritized replay, n-step returns, and hindsight goals where valid.
- [ ] Use terminal success and simulated ticks as authoritative reward. Learned
  reachability, novelty, and auxiliary predictions may improve credit assignment
  but cannot redefine success.
- [ ] Use reverse curricula or frontier checkpoints for rare terminal events:
  learn near a proven or discovered success state, then progressively move the
  start boundary backward without baking the demonstration path into the goal.
- [ ] Maintain uncertainty and quality-diversity archives so exploration does
  not greedily collapse onto the locally fastest state when a slower or
  different state may have better continuation potential.
- [ ] Detect and archive generic invariant violations as discovery leads even
  when they do not satisfy the requested predicate.

### 4.3 Autonomous experiment loop

- [ ] Implement the closed loop: choose checkpoint and goal, sample or infer a
  policy, execute natively, append full transitions, update the learner, measure
  held-out behavior, and choose the next experiment.
- [ ] Budget campaigns in simulated ticks and report useful transitions per
  second, not just attempt count.
- [ ] Allow an LLM or human to define goals, inspect evidence, and choose research
  directions without requiring either to hand-author every candidate batch.
- [ ] Stop or fall back when uncertainty, validation error, or distribution shift
  makes learned ranking indistinguishable from noise.

**Gate 4:** under a fixed simulation budget, training on accumulated transitions
outperforms an unlearned action sampler and the current hand-authored mutation
families on held-out starts. Removing the learned model measurably removes the
gain.

## 5. Prove discovery, optimization, and transfer

These are distinct tests. Passing one does not imply the others.

### 5.1 Demonstration-assisted Ordon optimization

- [ ] Use the human/125-tick tape as optional demonstration data and checkpoint
  evidence, not as a mandatory path.
- [ ] Beat 125 ticks to `ordon_spring_load_committed` and cold-replay the exact
  exported tape five times with identical hashes and predicate evidence.
- [ ] Compare learned and unlearned methods under equal simulated-tick budgets.

### 5.2 Goal-only Ordon discovery

- [ ] From the same first-Link-control state, provide only the terminal Ordon
  predicate, action contract, and tick budget. Exclude incumbent inputs,
  hand-authored waypoints, Ordon-specific wall distances, and required
  intermediate locations.
- [ ] Demonstrate that the system can discover a successful route and improve it
  using retained experience. It need not initially beat the assisted incumbent;
  this test measures discovery rather than tape repair.

### 5.3 Held-out transfer

- [ ] Select at least one different map and objective after the stage/Skybook
  audit, with mechanics that overlap Ordon but geometry and actors that do not.
- [ ] Compare a shared pretrained learner with a from-scratch learner under the
  same local transition budget.
- [ ] Require the shared model to reduce samples to first success or reach a
  better final policy without map-specific hardcoding.

### 5.4 Precision interaction benchmark

- [ ] Select one representative audited task requiring a conjunction such as
  exact collision offset, actor/enemy interaction, and correctly timed action or
  pickup.
- [ ] Define the smallest read-only success predicate and start from a nearby
  deterministic checkpoint.
- [ ] Show that the action and observation contracts can express and learn the
  setup, then export and cold-replay the exact tape. This is one capability
  benchmark, not a Skybook replication program.

**Gate 5:** the framework has separately demonstrated route optimization,
goal-only discovery, cross-map transfer, and one high-precision interaction.
Every promoted result is an exact deterministic tape.

## What we explicitly are not building yet

- A blind Skybook glitch replicator.
- A general-purpose visualization suite unrelated to a measured learning or
  debugging need.
- Route-specific sensors, waypoints, rewards, or action scripts presented as
  general intelligence.
- A distributed cluster before persistent checkpoints, renderless equivalence,
  observation cost, and native instance scaling are measured.
- Gameplay patches that make an exploit easier or manufacture proof.
- A commitment to DQN, DDQN, or any model label independent of held-out results.

## Definition of this plan's success

The framework is better than frame-by-frame human iteration when it can reuse
prior transitions to discover or improve exact inputs under a fixed simulation
budget, transfer useful mechanics to a held-out map, solve a precision
interaction without being given its recipe, and replay every claimed result
deterministically from a clean supported start.
