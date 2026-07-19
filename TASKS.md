# Core automation harness backlog

This is the active queue for one useful harness loop:

1. boot directly into a small test scenario;
2. query only the state needed by its objective;
3. let scripted, random, search, or learned agents choose inputs;
4. record comparable objective and episode evidence;
5. narrow proposals to a winner; and
6. prove that winner by cold-replaying its realized tape.

Talking to one NPC or picking up one rock is as valuable here as reproducing a
glitch. The current milestone is infrastructure confidence, not exhaustive
game coverage or autonomous glitch discovery. Difficult, speculative, and
whole-game work lives in [`TASKS_DEFERRED.md`](TASKS_DEFERRED.md).

## Queue rules

- `[x]` means the stated boundary exists and has direct test evidence.
- Add a fact, action, or observer only when a checked-in objective needs it.
- Missing state stays explicitly missing; it must never become false or
  success by accident.
- An agent, learner, or search algorithm proposes. Only semantic objective
  proof and an independently replayable absolute tape can win.
- Every comparison binds the exact scenario, build, objective, observation,
  action, settings, seed, and proposer identities needed to reproduce it.
- Ordinary harness runs are read-only with respect to gameplay state.
- Put status and design notes under `docs/glitch-hunting/`; keep this file as
  the short active queue.

## 1. Cheap end-to-end objectives

- [x] Define a versioned suite case that binds boot, scenario, objective,
  required facts, seed input, budget, and expected terminal class.
- [x] Check in a stage-ready case that boots directly into a map and proves
  player/location readiness without controller input.
- [x] Check in a reach-point case that moves Link into a bounded region and
  proves stable arrival.
- [x] Check in a talk-to-NPC case with an exact placed-NPC selector and proof
  of the expected dialogue or event edge.
- [x] Check in a pick-up-object case with an exact placed-object selector and
  proof that Link carries or owns that object.
- [x] Add a cheap negative control beside each positive case so nearby
  position, wrong target, or missed interaction cannot satisfy the objective.
- [x] Run the four cases twice through one public macOS command and retain one
  machine-readable conformance report.

**Done when:** these small cases exercise boot, control, query, objective, and
evidence collection without a debug write, manual mid-run action, or visual
judgment.

## 2. One execution and episode boundary

- [x] Preserve exact four-port controller state in canonical `DUSKTAPE`, with
  exclusive ownership and realized-tape output.
- [x] Support process boot and explicit stage/room/point/layer boot, including
  an optional named save/loadout fixture and a readiness gate.
- [x] Compile bounded static and reactive controllers while retaining an
  ordinary absolute tape as replay authority.
- [x] Define one authenticated run request and one typed run result covering
  the executable, game data, scenario, objective, observations, actions, seed,
  budgets, fidelity, evidence, and terminal reason.
- [x] Route tape playback, reactive control, the active population search, and
  learned proposals through that same request/result executor.
- [x] Give a controller one versioned pre-input observation and require one
  bounded action response without game-state mutation authority.
- [x] Enforce the logical-tick budget independently of host timeout and retain
  a precise terminal reason plus authenticated partial artifacts on failure.
- [x] Define one immutable episode artifact aligning pre-input state, consumed
  action, post-simulation state, objective/reward evidence, terminal reason,
  realized tape, and lineage.

**Done when:** random, scripted, search, and learned agents can attempt the same
objective contract and produce directly comparable episode results.

## 3. Minimum objective and state-query surface

- [x] Compile deterministic objective predicates with Boolean composition,
  comparisons, ranges, regions, stability, sequences, exact actor selectors,
  flags, events, and named projections.
- [x] Evaluate facts at one immutable post-simulation boundary and retain
  first-hit evidence plus objective progress.
- [x] Keep objective truth separate from reward shaping and proposer scores.
- [x] Require declared observation families and return unsupported when their
  evidence is unavailable or incomplete.
- [x] Expose stable reached, exhausted, impossible, unsupported, timeout,
  crash, hang, target-lost, and nondeterministic terminal classes.
- [x] Inspect objective source, compiled identity, required facts, progress,
  first hit, and missing evidence from the CLI.
- [x] Define one versioned typed-fact response shared by live control, traces,
  objective evaluation, and offline inspection, including explicit missingness
  and source phase.
- [x] Finish only the player, interaction, dialogue/event, carry/ownership,
  exact placed-actor, and local spatial facts required by the four cases.
- [x] Prove native and offline evaluation agree for every fact and objective
  used by those cases.

**Done when:** the four objectives see the same bounded facts online and
offline, can explain missing evidence, and cannot confuse the wrong actor with
the authored target.

## 4. Small learning and proposal loop

- [x] Provide deterministic nearest-neighbor, tabular, and tree-FQI baselines
  over immutable transition batches.
- [x] Provide structured candidate mutation and bounded search strategies with
  pre-evaluation deduplication and per-proposer budget accounting.
- [x] Define one candidate envelope for scripted, random, structured-search,
  and learned proposals with exact parent, objective, action schema, seed, and
  proposer identity.
- [x] Freeze one dataset generation, train one simple baseline, and emit only
  bounded candidate proposals; the learner cannot launch runs or claim proof.
- [x] Collect successes, ordinary failures, and near misses for one cheap
  objective, keeping evaluation episodes out of training until evaluation
  closes.
- [x] Compare the baseline against scripted/random/structured proposals under
  the same native objective and simulator budget.
- [x] Demonstrate on fixed seeds that the learned proposer improves native
  proposal ordering over the best non-learned lane.
- [x] Stop or fall back to structured proposals when required facts, action
  support, determinism, or basic held-out performance are inadequate.

**Done when:** one simple learner reproducibly improves proposal ordering for
one cheap objective without receiving a separate execution or proof path.

## 5. Narrow proposals into proved winners

- [x] Rank candidates lexicographically by objective feasibility, progress,
  first-hit tick, tape length/input complexity, and measured risk; unknown risk
  must remain unknown.
- [x] Preserve materially different successful boundary states long enough to
  avoid narrowing onto a locally cheap but unusable result.
- [x] Minimize a finalist only while preserving objective evidence, boot,
  fidelity, and terminal boundary class.
- [x] Cold-replay the realized tape at least twice with no controller or model
  in the loop before calling it a winner.
- [x] Keep contradictory replays and quarantine that candidate/build/scenario
  combination instead of averaging the contradiction away.
- [x] Emit a compact report comparing budget, objective hits, useful boundary
  states, replay verdict, and best proved tape per proposer.

**Done when:** the reported winner is a minimized, repeated, replayable tape,
not a learner score, controller claim, or lucky filename.

## 6. Minimal operator and reproducibility path

- [x] Define operation-specific compatibility and provide `huntctl identity
  compare` with precise incompatibility output.
- [x] Reject incompatible inputs automatically at the active run, episode,
  comparison, and replay boundaries.
- [x] Add one top-level command that resolves a suite case, runs selected
  proposers, ranks results, cold-replays finalists, and writes the report.
- [x] Support a dry run showing resolved paths, identities, required facts,
  capabilities, budgets, and output locations.
- [x] Keep generated evidence under an ignored build root and print the first
  unsupported or mismatched fact with its artifact path.
- [x] Document the shortest macOS loop: inspect case, run, inspect evidence,
  replay finalist, and explicitly keep or discard it.

**Done when:** a developer can run and diagnose the cheap-objective loop from a
clean macOS checkout without hand-assembling algorithm-specific commands.

## Immediate implementation order

1. [x] Finish the talk-to-NPC and pick-up-object facts and fixtures.
2. [x] Route search and learned candidates through the authenticated executor.
3. [x] Seal the shared episode artifact and pre-input observation/action turn.
4. [x] Run scripted, random, structured, and one simple learned proposer on one
   cheap objective under the same budget.
5. [ ] Rank, minimize, and cold-replay the finalists.
6. [x] Publish one conformance/tournament report and use its measured friction
   to decide what, if anything, returns from `TASKS_DEFERRED.md`.
