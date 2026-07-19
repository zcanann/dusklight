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
5. [x] Rank, minimize, and cold-replay the finalists.
6. [x] Publish one conformance/tournament report and use its measured friction
   to decide what, if anything, returns from `TASKS_DEFERRED.md`.

## 7. Exact anchored-route finalist minimization

- [x] Add `huntctl search minimize-route` for a proved anchored suffix. Reduce
  actions and durations only through the authenticated clean-boot evaluator,
  preserving the exact objective identity, source boundary, goal tick, and
  terminal boundary fingerprint across repeated processes.
- [x] Minimize the learned Ordon Spring finalist and retain a machine-readable
  source proof, reduction history, final proof, and compact realized tape.

**Done when:** route and glitch finalists can be reduced without launching a
general search or silently changing the successful terminal state.

## 8. Compiler-enforced finalist-reduction ownership

- [x] Move boot and anchored-route minimization policy out of
  `dusklight-evaluation` and into `dusklight-orchestration`; the reducer may
  depend on authenticated evaluation, but evaluation must not depend back on
  reducer policy.
- [x] Preserve the public CLI surface through compatibility re-exports, ratchet
  the closed crate inventories, and pass the full workspace test suite.

**Done when:** adding another finalist reducer cannot grow the evaluator crate
or acquire a second native proof path by mere module placement.

## 9. Compiler-enforced bounded-search ownership

- [x] Move ordinary, beam, continuous, and Bayesian native search drivers from
  `dusklight-evaluation` into `dusklight-orchestration`; search policy may call
  evaluation, but evaluation must not own optimizer loops.
- [x] Move pure parent/child tape intervention discovery into
  `dusklight-search`, preserve the CLI facade, ratchet closed inventories, and
  pass the full workspace suite.

**Done when:** adding another bounded optimizer cannot grow the evaluator crate
or hide portable candidate logic inside native-execution policy.

## 10. Compiler-enforced tournament ownership

- [x] Move proposer tournament selection, budget accounting, and comparison
  orchestration out of `dusklight-evaluation`; retain learned-proposal fact and
  holdout admission as explicit evaluator policy.
- [x] Preserve equal-budget and campaign CLI behavior, ratchet the closed crate
  inventories, and pass the full workspace suite.

**Done when:** tournament policy depends one-way on authenticated evaluation,
while evaluator admission remains independent of tournament coordination.

## 11. Recoverable exact route minimization

- [x] Checkpoint `minimize-route` after every completed reduction round and add
  `--resume` for an interrupted output root. Re-authenticate the objective and
  reprove both the source and retained candidate before spending new proposals.
- [x] Refuse stale or tampered checkpoints, never overwrite partial evidence,
  preserve the original candidate budget and exact terminal contract, and test
  the resume state machine.

**Done when:** interrupting a long route reduction no longer discards completed
proposal work or weakens its clean-process proof requirements.

## 12. Compiler-enforced policy contract ownership

- [x] Move finalist-reduction config and result contracts from
  `dusklight-evaluation` beside their reducers in `dusklight-orchestration`.
- [x] Move specialized bounded-search and proposer-tournament config and result contracts
  beside their orchestration policy, preserving the public CLI facade.
- [x] Ratchet the closed crate inventories and pass the full workspace suite.

**Done when:** adding a reducer, optimizer, or tournament policy cannot grow the
authenticated evaluator crate merely to define that policy's command surface.

## 13. Compiler-enforced anchored-generation ownership

- [x] Move the anchored generation, archive, learning-admission, evolution, and
  champion-promotion loop from `dusklight-evaluation` to
  `dusklight-orchestration` with its config and result contracts.
- [x] Expose only prepared evaluation, evidence-derived novelty/context, fact
  support, holdout admission, and authenticated suffix realization APIs.
- [x] Add direct one-way domain dependencies, ratchet inventories and line
  budgets, preserve the CLI facade, and pass the full workspace suite.

**Done when:** `dusklight-evaluation` can authenticate and interpret a prepared
population but cannot own a multi-generation search or learning loop.

## 14. Deterministic native worker assignment

- [x] Assign each planned trial to a stable worker lane independent of thread
  wakeup and completion order for ordinary and anchored evaluation.
- [x] Preserve parallel execution and fail-fast cancellation, prove exact lane
  coverage without duplicates, and pass the full workspace suite.

**Done when:** a healthy evaluation with the same population, repetition count,
and worker count cannot change evidence worker identities due to host timing.

## 15. Authenticated native worker schedule

- [x] Write a deterministic prelaunch worker schedule for ordinary and anchored
  evaluation, and link it from the evaluation report.
- [x] Validate completed trial identities and worker claims against that
  schedule before aggregation, with unit, CLI, and full workspace coverage.

**Done when:** native attempt evidence cannot silently disagree with its
predeclared worker lane or introduce an unplanned or duplicate trial identity.

## 16. Compiler-enforced evaluation-plan ownership

- [x] Move deterministic trial scheduling, lane iteration, schedule hashing,
  and completed-claim validation into a dependency-minimal crate.
- [x] Make native evaluation consume the exact declared schedule when launching
  lanes, ratchet the evaluator line budget, and pass the full workspace suite.

**Done when:** evaluator execution can consume a portable prelaunch plan but
cannot redefine worker assignment or validation policy in its coordination file.

## 17. Authenticated specialized bounded search

- [x] Carry one authenticated run-request template through every beam,
  CEM/CMA-ES, and Bayesian native evaluation batch.
- [x] Expose the same sole-authority CLI contract as ordinary search, reject
  mixed legacy/run-request inputs, and retain authenticated attempt evidence.

**Done when:** these bounded optimizers cannot drop an explicitly supplied core
harness authority between proposal generation and native ranking.

## 18. Authenticated boot finalist reduction

- [x] Carry an exact gameplay-ready run-request template through every boot
  minimization and timing-golf reduction batch and final proof.
- [x] Expose the shared sole-authority CLI contract, retain authenticated proof
  attempts, and reject mixed legacy/run-request inputs.

**Done when:** boot finalist tools cannot silently return to the legacy native
path after accepting a core-harness authority.

## 19. Authenticated anchored-route minimization

- [x] Require an optional run-request template to bind the exact anchored goal
  and milestone program, then retain it through every reduction and proof.
- [x] Bind that authority into resumable checkpoints, preserve legacy v1 resume
  only for legacy execution, and reject mixed or changed authority.

**Done when:** exact anchored finalist reduction and resume cannot drop or swap
an explicitly supplied core-harness authority.

## 20. Authenticated anchored proposer tournaments

- [x] Allow an anchored tournament to retain a sole run-request authority
  through its shared evaluation and cold-replay evidence.
- [x] Validate the request's exact milestone program and goal separately from
  the derived anchored-objective identity carried by proposal envelopes.

**Done when:** a fair anchored tournament can use the core harness without
conflating or weakening either authenticated identity.

## 21. Exact authority for anchored route search

- [x] Require `run-route --run-request` to bind the timeline-selected goal,
  compiled milestone program, and movement action schema exactly.
- [x] Share that validation across route search, reduction, and tournaments so
  anchored identity rules cannot drift between orchestration entry points.

**Done when:** accepting a sealed request cannot make an anchored route command
execute a different objective than the one it reports and ranks.

## 22. Dedicated finalist-reduction crate

- [x] Extract boot and anchored-route finalist reduction from the broad
  orchestration crate into a dedicated dependency-bounded crate.
- [x] Keep shared anchored request validation below orchestration, preserve the
  public API, and ratchet the new crate's exact source inventory and line caps.

**Done when:** finalist reduction cannot regain unrelated campaign, novelty, or
learning dependencies without an explicit architecture-policy change.

## 23. Dedicated bounded-search crate

- [x] Extract ordinary, beam, continuous, and Bayesian native search drivers
  from broad orchestration into a dedicated dependency-bounded crate.
- [x] Preserve anchored-search composition and the public CLI facade while
  ratcheting the new crate's exact inventory and dependency allowlist.

**Done when:** bounded optimizer loops cannot acquire campaign, novelty,
finalist-reduction, route, or workbench ownership by sibling-module imports.

## 24. Dedicated proposer-tournament crate

- [x] Extract equal-budget proposer selection, accounting, replay comparison,
  and finalist publication from broad orchestration into a dedicated crate.
- [x] Preserve campaign composition and the public CLI facade while ratcheting
  the new crate's exact inventory and dependency allowlist.

**Done when:** tournament policy cannot absorb campaign, objective-suite,
novelty, route, workbench, or CLI ownership through flat sibling imports.

## 25. Shared immutable episode store

- [x] Add one evidence-owned episode bundle index keyed by authenticated
  episode identity. Verify the manifest against its transition corpus and
  content-address the tape, trace, corpus, transition evidence, and manifest
  before publishing an immutable entry.
- [x] Let native population evaluation target an explicit shared episode-store
  root, retain per-run evidence paths for diagnosis, and prove two independent
  evaluation roots deduplicate the same bundle without accepting tampering.
- [x] Document the operator path, ratchet crate boundaries, and pass the full
  workspace suite.

**Done when:** independent search runs can share authenticated learning
episodes without copying mutable per-run paths or trusting a filename digest.

## 26. Verified episode retention and recoverable GC

- [x] Inventory and verify every immutable episode entry and referenced blob,
  rejecting malformed paths, identity drift, missing content, and corruption.
- [x] Retain episodes by authenticated episode ID and preview unretained entries
  plus unreachable blobs before moving them to an explicit recoverable trash
  root; refuse an empty retention set or a trash path inside the live store.
- [x] Expose the lifecycle through the CLI, document it, and pass the full
  workspace and architecture suites.

**Done when:** a shared episode store can be checked and compacted without
manual digest scraping, permanent deletion, or path-based trust.

## 27. Canonical active-state hash series

- [x] Derive one sealed semantic hash at every retained trace boundary from the
  validated typed-fact response, binding its exact field profile, phase,
  simulation tick, tape frame, boot origin, and source trace.
- [x] Compare two compatible series by boundary and report the first differing
  or missing boundary plus both hashes; reject profile, boot, tick-rate, seal,
  ordering, or source tampering before comparison.
- [x] Expose generation and comparison through `huntctl trace`, document the
  supported facts and whole-game non-claim, and pass the full suites.

**Done when:** parity and reset experiments get a cheap deterministic
divergence alarm over the active query aperture and can escalate to trace diff
at an exact boundary.

## 28. Domain-owned trace CLI adapter

- [x] Move trace inspection, typed facts, semantic hashes, timelines, and
  comparison commands out of the root binary into the dedicated CLI domain.
- [x] Lower the root-binary line ratchet and pass the trace integration plus
  architecture suites without changing the public command surface.

**Done when:** adding trace tooling cannot silently regrow the root Rust binary
as a flat implementation module.

## 29. Domain-owned corpus CLI adapter

- [x] Move tape-corpus operations plus content/episode-store lifecycle commands
  out of the root binary into one corpus CLI domain.
- [x] Ratchet the reduced root binary and pass corpus lifecycle, lint, and
  architecture gates without changing the public command surface.

**Done when:** corpus and immutable-store lifecycle growth cannot silently
accumulate in the root Rust binary.

## 30. Domain-owned world CLI adapter

- [x] Move world inventory, spatial indexing/query, and KCL/PLC inspection out
  of the root binary beside the existing world domain crate.
- [x] Ratchet the reduced root binary and pass real-fixture spatial, lint, and
  architecture gates without changing the public command surface.

**Done when:** static-world tooling can grow without turning the root binary
back into a cross-domain implementation file.

## 31. Domain-owned timeline CLI adapter

- [x] Move timeline inspection/rebasing, route-store lifecycle, thumbnail
  pruning, and workbench launch out of the root binary.
- [x] Ratchet the reduced root binary and pass timeline/store, lint, and
  architecture gates without changing the public command surface.

**Done when:** route and workbench command growth is owned beside those domains
instead of accumulating in the root Rust binary.

## 32. Domain-owned objective and control CLI adapters

- [x] Move fixture, observation, milestone, controller, and semantic/comparison
  oracle commands into explicit CLI domains.
- [x] Ratchet the reduced root binary and pass each public adapter integration,
  lint, and architecture gate without changing command behavior.

**Done when:** objective/control surface growth no longer accumulates as mixed
implementation code in the root Rust binary.

## 33. Domain-owned benchmark and identity CLI adapters

- [x] Move revision-pinned benchmark metadata and artifact-compatibility
  commands out of the root binary into explicit CLI domains.
- [x] Ratchet the reduced root binary and pass benchmark/identity integration,
  lint, and architecture gates without changing command behavior.

**Done when:** offline benchmark and compatibility tooling no longer depends on
mixed root-module implementation ownership.

## 34. Domain-owned harness and campaign CLI adapter

- [x] Move authenticated suite/request/result lifecycle, execution, inspection,
  and objective campaign dispatch out of the root binary.
- [x] Ratchet the reduced root binary and pass harness/campaign integration,
  lint, and architecture gates without weakening fail-closed behavior.

**Done when:** the core authenticated execution path has explicit CLI ownership
and the root binary cannot absorb harness policy implementation.

## 35. Domain-owned worker-control CLI adapter

- [x] Move worker hello/ping, deterministic pool health, and unavailable
  engine-session capability reporting out of the root binary.
- [x] Ratchet the reduced root binary and pass worker/pool integration, lint,
  and architecture gates without changing protocol behavior.

**Done when:** persistent-control bootstrap has explicit CLI ownership and
`main.rs` contains no production command implementation.

## 36. No ambient production wildcard dependencies

- [x] Replace root-wide CLI imports and cross-crate evaluation wildcards with
  explicit helper/type imports at each owning module.
- [x] Reject future `crate::*` CLI imports and external-crate wildcards in the
  architecture policy; pass full workspace, lint, and boundary gates.

**Done when:** moving or deleting an unrelated root/evaluation import cannot
silently change which names another production domain receives.

## 37. Authenticated cold-process throughput baseline

- [x] Benchmark repeated isolated executions of one sealed run request and
  retain each exact request/result identity, terminal, tick count, native
  process time, and end-to-end harness time in one machine-readable report.
- [x] Reject incomparable attempts, expose logical-tick and candidate
  throughput from the CLI, document the macOS operator path, and measure one
  checked-in cheap objective before designing persistent reset/session work.

**Done when:** a future persistent worker or reset optimization can be compared
against a reproducible process-per-run baseline without weakening run proof.
