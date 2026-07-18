# Core automation harness backlog

This is the active queue for making Dusklight a trustworthy harness for
controlling agents, expressing objectives, querying game state, collecting
learning evidence, and narrowing proposals into proved winners.

Cheap objective tests are the primary development targets. Reaching a point,
talking to an NPC, picking up a rock, or triggering a door is valuable when it
proves the complete harness. Glitch reproduction is not privileged over these
tests. Ambitious research and exhaustive coverage live in
[`TASKS_DEFERRED.md`](TASKS_DEFERRED.md) and are not active obligations.

## Queue rules

- `[x]` means the stated boundary exists and has direct test evidence.
- `[ ]` is active only when its prerequisite above it is satisfied.
- Add observations, actions, and infrastructure for a checked-in objective,
  not for hypothetical whole-game completeness.
- Every result must retain exact scenario, build, objective, observation,
  action, and settings identity.
- A learner or reactive controller proposes. Only a realized absolute tape,
  semantic objective proof, and independent cold replay can win.
- Missing state remains explicitly missing. Native pointers and session-local
  process IDs are diagnostics, never portable identity.
- Ordinary playback and learning runs are read-only with respect to gameplay
  state. Experimental writes remain outside the active harness.
- Put architecture, status, evidence, and investigation notes under
  `docs/glitch-hunting/`; keep this file as tasks only.

## 1. Immediate objective conformance suite

- [x] Define a small versioned suite manifest binding each case to its stage or
  process boot, scenario fixture, objective program, required query facts,
  controller/tape seed, time budget, and expected terminal class.
- [x] Check in a **stage-ready** case that boots directly into a map and proves
  the declared player/location readiness condition without controller input.
- [x] Check in a **reach-point** case that moves Link into a bounded region and
  proves position plus stability for several logical ticks.
- [ ] Check in a **talk-to-NPC** case that selects one placed NPC, approaches it,
  presses the interaction input, and proves the expected event/dialogue edge.
- [ ] Check in a **pick-up-object** case that selects one placed carryable
  object, approaches it, presses the interaction input, and proves Link owns or
  carries that exact object.
- [ ] Add one ordinary negative case for each objective so a nearby position,
  wrong NPC, missed interaction, or wrong object cannot satisfy the oracle.
- [ ] Run every case from its declared boot at least twice and require identical
  terminal class, first-hit tick, realized tape, and selected boundary facts.
- [ ] Emit one machine-readable suite report with pass, objective miss,
  unsupported observation, capability mismatch, timeout, crash, and
  nondeterminism as distinct outcomes.
- [ ] Make the suite runnable with one documented `huntctl` command on macOS.

**Acceptance:** the four positive cases and their negative controls run through
the same public harness path. No case relies on a debug memory write, a manual
mid-run action, a filename convention, or visual-only judgment.

## 2. Agent execution and control

- [x] Preserve exact four-port controller state in canonical `DUSKTAPE`, with
  exclusive input ownership and a realized-tape output boundary.
- [x] Support process boot and explicit stage/room/point/layer boot, including
  an optional named save/loadout fixture and a readiness gate before tick zero.
- [x] Compile bounded static and reactive controller programs while retaining
  an ordinary absolute tape as replay authority.
- [x] Define one run request that binds the executable, game data, scenario,
  objective, observation view, action/controller schema, seed, logical-tick
  budget, fidelity mode, and artifact destination.
- [x] Define one run result with typed terminal reason, objective evidence,
  realized input, trace references, boundary fingerprint, timing counters, and
  complete identity.
- [ ] Route tape playback, reactive control, search evaluation, and learned
  proposals through that same request/result boundary.
- [ ] Give a controller a versioned pre-input observation and require one
  bounded action response without granting it game-state mutation authority.
- [ ] Enforce logical-tick budgets independently of host timeout; distinguish
  cancellation, worker crash, protocol failure, objective miss, and hang.
- [ ] Keep worker-pool scheduling deterministic for a fixed request sequence and
  seed, including stable result ordering independent of process completion.
- [ ] Retain partial artifacts after timeout or crash without treating them as
  complete evidence.

**Acceptance:** a random agent, a scripted controller, and a learned proposer
can execute the same objective contract, and each successful episode produces
an independently replayable tape.

## 3. Objectives, predicates, and terminal semantics

- [x] Compile a versioned predicate language with Boolean composition,
  comparisons, ranges, regions, stability, sequences, exact actor selectors,
  flags, events, and named value projections.
- [x] Evaluate objective facts at one immutable post-simulation boundary and
  retain first-hit evidence plus objective progress in the gameplay trace.
- [x] Add typed facts required by the conformance suite for interaction
  availability, dialogue/event start, carried/held actor identity, and object
  ownership changes.
- [x] Define objective programs separately from reward shaping and proposer
  scores; only objective truth determines success.
- [x] Require each objective to declare its necessary observation families and
  fail as unsupported when a family is unavailable or truncated.
- [ ] Return stable terminal classes for reached, exhausted, impossible,
  unsupported, timeout, crash, hang, target-lost, and nondeterministic.
- [ ] Add an inspection command that prints the source objective, compiled
  identity, required facts, current progress, first-hit boundary, and missing
  evidence in human-readable form.
- [ ] Add truth-table and native/offline parity tests for every fact used by the
  four conformance objectives.

**Acceptance:** objective evaluation is deterministic, inspectable, independent
of learner score, and cannot turn missing data into success.

## 4. Read-only state query boundary

- [ ] Define a versioned typed fact schema shared by live queries, trace
  channels, objective evaluation, observation views, and offline inspection.
- [ ] Define query status as present, absent, unavailable, truncated, stale, or
  invalid; include source phase and schema identity in every response.
- [ ] Surface the minimum player facts for the conformance suite: stage/room,
  position and prior position, velocity, facing, procedure/action state,
  interaction state, event/dialogue state, and carried actor identity.
- [ ] Surface bounded placed-actor facts: portable placement identity, type,
  room, transform, procedure/status, interaction eligibility, carry ownership,
  and target/parent/owner relationships needed by the selected objective.
- [ ] Provide exact placed selectors and explicit target loss; never silently
  fall back to a nearest actor when an exact target disappears.
- [ ] Add only the local collision/contact and static placement facts needed to
  navigate to the NPC and carryable object.
- [ ] Compile objective-specific observation views with stable feature order,
  explicit masks, units, ranges, categorical domains, and a content digest.
- [ ] Report per-query and per-view capture cost, bounded result capacity, and
  truncation.
- [ ] Keep query adapters out of gameplay implementation files except for the
  smallest compile-gated read aperture; audit every invoked native helper as
  side-effect free.
- [ ] Prove observer-off builds erase the query boundary and observer-on A/B
  runs preserve input, RNG, events, objective result, and selected state.

**Acceptance:** the conformance objectives use the same typed facts online and
offline, with no gameplay-visible write, allocation surprise, or guessed value.

## 5. Episode evidence and learning loop

- [ ] Define one immutable episode artifact binding run request/result,
  objective, realized tape, observations, actions, rewards, terminal reason,
  build/scenario identity, seed, worker, and lineage.
- [ ] Align every transition as pre-input state, consumed action, option
  duration, post-simulation state, objective/reward evidence, and terminal or
  truncation reason.
- [ ] Store episodes content-addressably and deduplicate exact repeats without
  erasing repetition evidence.
- [ ] Split train/validation/test data by episode and starting boundary, never
  by adjacent frames; fit normalization on training data only.
- [ ] Collect explicit failure and near-miss episodes for each conformance
  objective instead of training only on successful demonstrations.
- [ ] Publish state, action, procedure, spatial-phase, success, and missingness
  coverage before training.
- [x] Provide deterministic nearest-neighbor, tabular, and tree-FQI baselines
  over immutable transition batches.
- [ ] Define one learner adapter that consumes a frozen dataset generation and
  emits a versioned model plus bounded proposal requests; it cannot launch the
  game or promote artifacts.
- [ ] Bind model identity to feature/action/objective schemas, normalization,
  code/data builds, corpus generation, seed, optimizer configuration, and
  exact model bytes.
- [ ] Evaluate learned proposals on isolated native episodes that cannot enter
  training until the evaluation generation closes.
- [ ] Stop or fall back to scripted/structured proposals when coverage,
  calibration, determinism, or supported-action checks fail.

**Acceptance:** one baseline learner can improve proposal ordering for at least
one trivial objective without owning execution or promotion, and the comparison
is reproducible from immutable artifacts.

## 6. Proposal search and narrowing winners

- [x] Support exact tape slicing/layering/diffing, deletion minimization,
  coordinate and timing golf, structured tactic mutation, beam search, CEM,
  CMA-ES, and Bayesian bounded search.
- [x] Deduplicate candidates before native evaluation and attribute simulator
  budget, improvements, misses, crashes, and duplicates to each proposer.
- [ ] Define one candidate envelope for scripted, random, structured-search,
  archive, and learned proposals with exact parent, objective, action schema,
  seed, and proposer identity.
- [ ] Rank candidates lexicographically by objective feasibility, progress,
  first-hit tick, tape length/input complexity, measured risk, and compatible
  boundary state; unknown risk must not become zero.
- [ ] Preserve several non-dominated winners when they end in meaningfully
  different RNG, actor, loader, contact, or downstream boundary states.
- [ ] Minimize a winner only while preserving objective evidence, boot origin,
  fidelity, and terminal boundary class.
- [ ] Require repeated cold replay of the realized tape with no controller or
  model in the loop before promotion.
- [ ] Keep contradictory replays and quarantine the candidate/build/scenario
  combination instead of reporting a stability percentage.
- [ ] Attach a promoted winner to route history only through an explicit human
  action; unpromoted results remain disposable build artifacts.
- [ ] Emit a compact tournament report comparing wall time, simulator ticks,
  episodes, objective hits, unique useful boundaries, cold-replay passes, and
  best proved tape per proposer.

**Acceptance:** several proposal sources compete under the same budget and
native objective, and the reported winner is a minimized, repeated,
cold-replayable tape rather than a model output.

## 7. Identity and deterministic evidence

- [ ] Materialize complete portable identity in every run, episode, trace,
  dataset, model, candidate, comparison, and promotion artifact.
- [ ] Hash executable/Aurora revisions and dirty state; compiler,
  configuration, features, architecture, protocol capabilities, fidelity;
  game-data/region/language; scenario; objective; action/observation schemas;
  and simulation-relevant settings.
- [x] Define operation-specific compatibility for replay, trace merge, model
  training, checkpoint restore, and cross-build comparison.
- [x] Provide `huntctl identity compare` with complete-input validation and a
  human-readable list of every incompatible field.
- [ ] Apply compatibility checks automatically at every artifact-consuming CLI
  and workbench boundary; never infer identity from a configured path.
- [ ] Define a small canonical hash over the semantic facts used by the active
  objective, explicitly excluding padding, pointers, volatile process IDs, and
  unordered traversal.
- [ ] Store periodic hashes plus dense observations around a mismatch and print
  the first divergent boundary with typed fact/event differences.
- [ ] Compare repeated runs of each conformance case and quarantine any
  disagreement until its input, time, RNG, async, floating-point, observation,
  or initialization source is explained.
- [ ] Inventory and control only the game-visible clocks, loader completions,
  and RNG streams exercised by the active conformance cases.

**Acceptance:** incompatible evidence is rejected with a precise explanation,
and identical requests either agree at every selected semantic boundary or
produce a localized divergence artifact.

## 8. CLI and operator workflow

- [ ] Add one top-level campaign command that validates an objective-suite case,
  launches its requested proposer(s), enforces budgets, stores episodes, ranks
  results, cold-replays finalists, and writes a summary artifact.
- [ ] Support dry-run inspection of all resolved paths, identities,
  capabilities, objective requirements, budgets, and output locations before a
  worker launches.
- [ ] Keep generated runs, models, traces, thumbnails, and reports under an
  ignored content-addressed build root; keep only authored fixtures,
  objectives, seed tapes/controllers, and approved route segments in Git.
- [ ] Print actionable failure messages with artifact paths and the first
  unsupported/mismatched fact instead of requiring log archaeology.
- [ ] Provide macOS VS Code launch/tasks alongside, without replacing Windows
  configuration, for building, running the conformance suite, inspecting a
  failure, and replaying a winner headfully.
- [ ] Document the shortest operator loop: select case, inspect contract, run
  campaign, inspect ranking/evidence, replay finalist, promote or discard.

**Acceptance:** a developer can exercise and diagnose the harness from a clean
macOS checkout without editing source, hand-assembling several commands, or
opening an algorithm-specific UI.

## 9. Verification and safety gates

- [ ] Unit-test every active schema's round trip, limits, unknown fields,
  truncation, corruption, noncanonical values, and content identity.
- [ ] Maintain independent Rust/native codec fixtures for tapes, controllers,
  objectives, traces, observations, and run messages.
- [ ] Add mock-worker integration tests for run lifecycle, budgets, crashes,
  partial artifacts, deterministic scheduling, and identity rejection.
- [ ] Add native tests proving the observer-off build has no query symbols or
  layout/control-flow changes and the observer-on hot path stays bounded and
  allocation-free.
- [ ] Run the objective conformance suite in process-boot and stage-boot modes
  where applicable, including positive and negative controls.
- [ ] Treat sanitizers, protocol fuzzing, path validation, artifact size caps,
  and recoverable cleanup as release gates for the harness-facing code.
- [ ] Keep CI fixtures free of copyrighted game data; real-disc acceptance runs
  remain local and report a clear skip when inputs are absent.

**Acceptance:** focused unit/integration tests and the four cheap objectives
prove the public harness boundary before any deferred research is reactivated.

## Immediate implementation order

1. [x] Write the objective-suite schema and stage-ready/reach-point fixtures.
2. [ ] Close the minimum interaction and carried-object query facts.
3. [ ] Add talk-to-NPC and pick-up-object fixtures with negative controls.
4. [ ] Unify their execution under the run request/result and episode artifact.
5. [ ] Run scripted, random, structured, and one baseline learned proposer
   through the same campaign/tournament boundary.
6. [ ] Cold-replay and minimize the winners; publish one conformance report.
7. [ ] Review measured friction before moving any item back from
   `TASKS_DEFERRED.md`.
