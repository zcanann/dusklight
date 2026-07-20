# Route-learning framework roadmap

This is the sole active task list. It contains only the work required to make
Dusklight optimize a deterministic movement segment better than a skilled human
editing inputs frame-by-frame. Completed capability inventory belongs in
`docs/glitch-hunting/status.md`.

The immediate proving ground is the authenticated Link-control-to-Ordon-Springs
segment. The current 128-tick route is visibly weak: its steering is noisy, its
line is not consistently straight, and its roll schedule has not been causally
optimized. The framework is not successful until it improves that route for
the right reasons and cold-replays the result exactly.

The last farm demonstrated why algorithm tuning alone is insufficient. It
launched 1,152 complete game processes, replayed the 440-tick boot prefix for
every 128-tick suffix, took roughly 45 minutes, and wrote 21,063 files totaling
4.44 GiB without finding a faster route. Its fitted-Q model learned from
completed open-loop tapes and proposed more open-loop frame overwrites. It
could not restore one state and compare alternative actions from that state.

## Completion gate

The roadmap is complete only when one checked campaign:

- boots each native worker and reaches Link control once;
- restores that exact state between candidate suffixes without relaunching or
  replaying the prefix;
- runs the per-tick observation, policy, input, and scoring loop in Dusklight;
- supplies generic movement, camera, curve, button, and roll tactics without
  embedding the Ordon route;
- learns from causal counterfactuals produced from identical restored states;
- keeps workers evaluating while training and proposal generation continue;
- substantially beats the best retained human/TAS Ordon tape under an equal
  start predicate and terminal predicate;
- realizes the winner as an ordinary absolute tape; and
- cold-replays the complete boot-to-terminal chain five times with identical
  per-tick state hashes and terminal evidence.

“Substantially” is not satisfied by input simplification or an equal-time tape.
The winner must save multiple simulation ticks and leave no obvious untested
earlier roll, straighter heading, shorter corner, or removable input window in
the locally exhaustive counterfactual audit.

## Invariants

- Replay divergence is a framework bug. Never compensate by searching for a
  more forgiving tape or averaging contradictory runs.
- Logical simulation ticks are the only time score. Process launch, rendering,
  shader compilation, audio, filesystem I/O, and host scheduling do not count.
- Normal automation observes gameplay read-only and controls only virtual PAD
  input. Checkpoint restoration is a compile-gated harness operation, not a
  gameplay mechanic or proof path.
- Absolute tapes remain replay authority. Checkpoints, controllers, models,
  rewards, and search scores may propose but cannot prove a result.
- One worker process owns one game address space. A crash or suspected memory
  corruption retires that worker; it may not restore itself and continue
  producing trusted samples.
- Every checkpoint, model, episode, and tape binds the executable, game data,
  scenario, objective, state/action schema, settings, prefix, RNG, and ancestry.
- Missing state is explicitly unavailable, never silently false or zero.
- A protocol stub, refusal report, capability flag, design document, or test of
  an unimplemented path does not complete a task.

## 1. Measure the real baseline

- [ ] Check in one command that reruns the current 128-tick Ordon campaign
  workload and reports process launches, prefix ticks, candidate suffix ticks,
  candidate-ticks/second, CPU utilization, learner time, simulator idle time,
  artifact time, file count, and bytes written.
- [ ] Record the current human/TAS incumbent's per-tick position, velocity,
  facing, procedure, camera, collision correction, applied input, button edges,
  roll state, and objective progress.
- [ ] Derive route diagnostics: distance traveled, displacement toward the load
  zone, path curvature, heading error, collision loss, corner duration, speed
  profile, roll initiation/recovery, roll spacing, and terminal overshoot.
- [ ] Separate process startup, engine setup, prefix replay, checkpoint/restore,
  useful suffix simulation, inference, training, serialization, and disk I/O in
  every later comparison.

**Done when:** every architectural change can be compared against an unchanged,
reproducible workload and the route's visible deficiencies are numeric rather
than subjective.

## 2. Persistent native episode execution

- [ ] Replace the health-only worker with a native engine session supporting
  `load`, `run_to_boundary`, `checkpoint`, `upload_program`, `upload_model`,
  `run_batch`, `cancel`, `health`, and `shutdown`.
- [ ] Keep the process, disc image, Aurora, immutable resources, shader cache,
  and process-lifetime services alive across candidate batches.
- [ ] Make Dusklight advance one logical tick or one bounded episode without
  returning through game/process teardown.
- [ ] Reset automation-owned predicates, controllers, tapes, observations,
  clocks, counters, recorders, result buffers, and error state between episodes.
- [ ] Upload immutable tape, controller, and model blobs once and address them
  by content identity in later batch requests.
- [ ] Return compact episode results in memory. Do not create routine per-run
  stdout, stderr, JSON, database, memory-card, configuration, or trace files.
- [ ] Retain the existing cold process executor independently for conformance
  and promotion.
- [ ] Add heartbeat, timeout, cancellation, crash classification, and automatic
  clean worker replacement.

**Done when:** one worker completes 1,000 short episodes in one process with
bounded memory growth and agrees with the cold executor on the conformance set.

## 3. Exact Link-control checkpoint

- [ ] Capture the first checkpoint only at the authenticated pre-input
  `link_control` boundary used by the Ordon segment.
- [ ] Require a quiescent boundary: no unsafe stage load, DVD/ARAM transfer,
  audio callback, movie task, render submission, or host job may be in flight.
- [ ] Capture and restore MEM1, ARAM, heap metadata, game RNG, logical clocks,
  PAD state, and automation state required to resume at the exact next input.
- [ ] Inventory future-affecting mutable globals and host allocations outside
  MEM1/ARAM. Register or isolate them explicitly; the current typed-fact hash is
  not sufficient evidence of complete state.
- [ ] Bind checkpoint identity to executable, game data, full prefix tape,
  source predicate and fingerprint, tick boundary, memory-layout version,
  settings, and a manifest of captured state.
- [ ] Refuse capture or restore when quiescence or state coverage cannot be
  established. The first checkpoint is same-process and build-bound; portable
  arbitrary savestates are not required here.
- [ ] Prove A/B/A restoration: run suffix A, restore and run B, restore and run
  A again, then require A1 and A2 to match at every per-tick state hash.
- [ ] Compare restored runs with fresh-process full-prefix runs over movement,
  rolls, collision, RNG, events, and the Ordon stage transition.
- [ ] Run randomized deterministic sentinel episodes throughout a farm and
  retire the worker on the first mismatch, retaining the first divergent tick.
- [ ] Measure full-copy restoration first; add dirty-page restoration only if
  measurement shows it is needed after parity is proven.

**Done when:** 1,000 randomized A/B/A cycles and the cold comparison set have
zero divergence, and the 440-tick prefix executes once per worker rather than
once per candidate.

## 4. Native policy and tactic runtime

The toolbox supplies generic controllable operations. It must not contain the
answer to the benchmark. Learning chooses their parameters, timing,
composition, initiation, and termination.

- [ ] Execute the stateful policy loop at Dusklight's pre-input boundary with
  no per-tick Rust IPC, filesystem access, or gameplay write.
- [ ] Support bounded state, conditions, option initiation/termination,
  timeouts, and deterministic composition of main stick, camera stick,
  triggers, and buttons.
- [ ] Always emit the exact realized four-port tape for independent playback.
- [ ] Preserve exact raw GameCube pad states, button edges/holds/releases,
  analog values, neutral, duration, and ownership as the lowest-level actions.
- [ ] Add a generic pulse operator with searchable start, period, phase, hold,
  count, and stop condition. It may drive A, L, or any button combination while
  other controller layers continue.
- [ ] Add generic world-, player-, and camera-relative heading control.
- [ ] Add seek-to-coordinate/offset/opening and maintain-distance controllers
  with configurable gain, magnitude, stop, and overshoot behavior.
- [ ] Add camera-to-heading control that can compensate the main stick to
  preserve a requested world trajectory while the camera turns.
- [ ] Add line, piecewise, Bézier, Catmull-Rom, and waypoint paths with
  searchable control points, duration, sampling phase, and feedback strength.
- [ ] Add a parameterized roll option with a single A edge, chosen heading,
  initiation state, recovery behavior, and termination. Do not model a roll as
  holding A across an arbitrary frame window.
- [ ] Keep raw-pad actions available beside every semantic tactic so an
  incorrect abstraction cannot hide a useful input.
- [ ] Run frozen deterministic model inference inside the worker, switching
  model versions only at declared episode boundaries.

**Done when:** a worker can run raw, scripted, and learned policies through the
same tick loop, and no checked tactic contains an Ordon-specific coordinate,
heading, corner tick, or roll spacing.

## 5. State and spatial facts required for movement learning

- [ ] Define one versioned movement state containing Link position, velocity,
  acceleration, speed, facing, procedure/subprocedure, action/animation phase,
  prior input, camera pose, collision correction, event state, transition
  state, relevant timers, and complete RNG identity.
- [ ] Expose action eligibility and timing facts: ticks since relevant button
  edges, roll initiation, recovery, contact, targeting, and option termination.
- [ ] Express target direction in world, player, camera, and velocity frames so
  the learner need not infer coordinate transforms from sparse outcomes.
- [ ] Expose the authored load-zone/opening geometry and read-only local queries
  needed to reach it: raycast, sweep, clearance, nearest surface, signed
  distance, predicted contact, and collision-aware distance.
- [ ] Build immutable map geometry indices once and reuse them. Do not place
  every polygon, tree, rock, or actor into every observation vector.
- [ ] Retain feature missingness, units, coordinate frame, normalization,
  query cost, and phase in the state schema identity.
- [ ] Detect observation aliasing by finding identical states with different
  checkpoint-backed action outcomes. Add the missing fact or bounded policy
  memory rather than asking the critic to average hidden state.

**Done when:** counterfactual outcomes are predictable from the declared state
or produce a concrete retained state-aliasing report; framework nondeterminism
is never treated as learnable noise.

## 6. Causal transitions and useful rewards

- [ ] Restore one checkpointed state and try alternative raw inputs, tactic
  types, headings, curve parameters, durations, roll ticks, and terminations.
- [ ] Retain checkpoint and sibling-group identity so training can use exact
  paired comparisons instead of correlations between unrelated tapes.
- [ ] Keep the semantic terminal predicate and first-hit tick authoritative.
  Training reward is separate and fully decomposed in episode evidence.
- [ ] Use a base `-1` tick cost plus auditable potential-based shaping from
  collision-aware progress to the goal. Do not reward visual straightness
  directly; shorter collision-valid motion should create the signal.
- [ ] Report progress, collision loss, option cost, and terminal adjustment
  independently so reward mistakes and hacking are visible.
- [ ] Treat restored RNG as state. Identical checkpoint/action divergence is a
  reset failure, not variance for Q-learning to smooth away.
- [ ] Split training and evaluation by checkpoint/state region rather than
  allowing adjacent frames or sibling actions to leak across both sets.

**Done when:** on held-out restored states, learned value ordering identifies
the better straight-line, corner, and roll-timing alternatives more often than
random or frequency ordering, and native rollout confirms the comparison.

## 7. Search and learning loop

- [ ] Replace generation-wide evaluate/refit pauses with persistent workers
  that continuously request episodes while proposal generation and training
  proceed asynchronously.
- [ ] Maintain an online replay buffer with immutable model generations,
  bounded worker-policy staleness, checkpoint identity, and causal sibling
  groups.
- [ ] Establish equal-useful-candidate-tick baselines for human tape mutation,
  structured tactic search, continuous optimization, and learned proposals.
- [ ] Add Double-Q with target networks, prioritized replay, n-step returns,
  ensemble uncertainty, and distributional values as individually measured
  components. Do not add an RL acronym without an Ordon ablation.
- [ ] Represent decisions as tactic type plus duration and parameters rather
  than only 16 quantized raw headings crossed with button holds.
- [ ] Run CEM/CMA-ES or Bayesian optimization over continuous headings, curve
  points, gains, corner timing, and roll schedules.
- [ ] Add short checkpoint-backed beam lookahead so useful action sequences can
  be evaluated without requiring one mutation to repair the entire later tape.
- [ ] Preserve materially different successful boundary states and RNG
  lineages instead of retaining only the current fastest result.
- [ ] Detect unsupported-action extrapolation, critic disagreement, policy
  collapse, OOD states, reward hacking, and held-out regression before granting
  a learned proposer more simulator budget.
- [ ] Profile and remove repeated full-model refits, corpus redecoding,
  quadratic proposal ranking, and other periods where all simulators sit idle.
- [ ] Choose worker count and affinity from measured useful throughput on the
  host; do not encode 16 as a framework limit.

**Done when:** workers spend at least 85% of scheduled lane time in useful
simulation or native inference, and a learned lane beats its non-learned
equal-budget baselines on held-out Ordon states.

## 8. Artifact, failure, and promotion path

- [ ] Use a bounded binary batch protocol or shared-memory rings for hot data.
  Upload programs/models once and return observations by rollout, never by tick.
- [ ] Keep discovery results compact: score, terminal, checkpoint/model/action
  identities, minimal trajectory, and failure classification.
- [ ] Materialize full traces, databases, screenshots, logs, and repeated proof
  only for determinism sentinels, anomalies, finalists, or explicit inspection.
- [ ] Content-address optional large artifacts and enforce per-campaign file and
  byte budgets.
- [ ] Immediately retire a worker after access violation, sanitizer/guard hit,
  allocator corruption, checkpoint mismatch, protocol corruption, or suspected
  memory overwrite. Preserve only evidence written before the last trusted
  boundary.
- [ ] Re-run suspicious or crashing inputs in a clean disposable process; one
  worker failure must not stop the scheduler or other lanes.
- [ ] Compile every finalist's realized actions into an ordinary absolute tape,
  then minimize only through exact native comparisons.
- [ ] Cold-run the complete prefix and suffix five times with no checkpoint,
  model, or reactive controller in the loop. Require identical state hashes,
  predicate evidence, first-hit tick, and terminal fingerprint.

**Done when:** the campaign's routine output is small, failures remain isolated,
and a promoted result is independently reproducible from process boot.

## 9. Ordon machine-versus-human proof

- [ ] Freeze the exact source and terminal predicates, fingerprints, score
  semantics, and allowed generic toolbox for the benchmark.
- [ ] Retain the 128-tick route and the best deliberate human/TAS revision as
  named baselines.
- [ ] Assert mechanically that campaign configuration and tactic definitions do
  not contain route-specific coordinates, headings, corner ticks, or roll
  spacing copied from either baseline.
- [ ] Run equal-budget ablations for raw mutation, structured tactics,
  continuous parameter search, learned value guidance, progress shaping, and
  checkpoint lookahead.
- [ ] For each improvement, retain its path, speed, heading, collision, camera,
  action, and roll timeline so the source of the frame win is explainable.
- [ ] Exhaustively test locally earlier roll/button edges, neighboring heading
  parameters, shorter tactic durations, and removable input windows around the
  final route.
- [ ] Require a multi-tick improvement rather than promoting an equal-time
  lower-complexity tape as success.
- [ ] Cold-prove the final full-chain tape five times and publish simulator
  budget, wall time, useful throughput, restore cost, ablations, route
  diagnostics, and exact proof identities.

**Done when:** the framework substantially beats the best retained human/TAS
Ordon route, explains the improvement, survives the local counterfactual audit,
and passes five identical cold full-chain proofs.

## Explicitly outside this roadmap

Do not start these merely because related scaffolding already exists:

- blind or withheld replication of the Skybook corpus;
- a farm visualization dashboard or multi-worker graphical compositor;
- deterministic multiplayer/network simulation;
- distributed or remote workers;
- general portable or arbitrary-tick savestates;
- exhaustive whole-game actor, polygon, renderer, audio, or memory queries;
- a general autonomous novelty/glitch-discovery campaign;
- every published RL algorithm, world model, graph encoder, or accelerator;
- cluster storage, dashboards, quotas, and broad artifact migration.

Reconsider one only after the Ordon completion gate exposes a concrete need.

## Immediate order

1. [ ] Baseline the current Ordon farm and route numerically.
2. [ ] Implement persistent native episode batches.
3. [ ] Implement and A/B/A-prove the exact Link-control checkpoint.
4. [ ] Run suffix batches without process, prefix, or routine-file tax.
5. [ ] Implement the native stateful tactic runtime.
6. [ ] Produce causal counterfactual transitions and progress rewards.
7. [ ] Run asynchronous structured, continuous, and learned competitors.
8. [ ] Substantially beat the human/TAS route.
9. [ ] Cold-prove and promote the winning absolute tape.
