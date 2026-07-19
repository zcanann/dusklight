# Deferred automation and glitch-research backlog

Nothing in this file is an active implementation obligation. Move an item into
`TASKS.md` only after a checked-in objective or measured harness bottleneck
demonstrates the need, and reduce it to the smallest testable slice when moving
it. Existing experimental code may remain useful without making its continued
expansion a priority.

## Deferred from the active core-harness pass

These are reasonable later improvements, but none is required to prove the
first cheap-objective learning and winner-narrowing loop.

- [ ] Deterministic parallel worker-pool scheduling, stable completion-order
  independence, remote workers, and throughput tuning beyond a sequentially
  reproducible evaluator.
- [ ] Content-addressed episode stores, cross-run deduplication, retention and
  garbage-collection policy, broad corpus publication, and historical schema
  migration.
- [ ] Full train/validation/test governance, normalization registries, broad
  state/action/procedure/spatial coverage reports, calibration suites, and OOD
  dashboards beyond the first baseline comparison.
- [ ] Multi-objective or whole-route tournaments, Pareto archives, exhaustive
  proposer combinations, automatic route-history integration, and campaign
  management beyond one explicitly selected objective.
- [x] Migrate every specialized legacy beam, CEM/CMA-ES, Bayesian, tournament,
  minimization, and golf entry point to the authenticated executor unless an
  active cheap-objective loop first demonstrates that it needs that algorithm.
- [ ] Complete identity materialization across every historical artifact and
  automatic compatibility enforcement in inactive legacy CLIs.
- [ ] Whole-game semantic hashing, periodic divergence capture, exhaustive
  clock/RNG/async attribution, and determinism quarantine beyond facts touched
  by active cases.
- [ ] General query cost accounting, schema browsing, arbitrary subscriptions,
  comprehensive capacity tuning, and observer A/B matrices beyond the narrow
  read-only aperture required by active objectives.
- [ ] A polished multi-platform VS Code workflow, full graphical diagnosis,
  release packaging, and a one-click operator experience beyond the documented
  macOS CLI loop.
- [ ] Exhaustive schema corruption suites, independent codec fixtures for
  every inactive format, broad fuzzing, sanitizer matrices, and long-running
  soak gates beyond focused tests of the public active boundary.

## Whole-corpus glitch reproduction

- [ ] Select and reproduce a small Skybook pilot after the basic objective
  conformance suite is reliable.
- [ ] Bind selected pages to exact Skybook revision/page digests, scenarios,
  preconditions, semantic oracles, known setups, fidelity, and proof ancestry.
- [ ] Build a selected-glitch mechanism/prerequisite graph only if a useful
  pilot grows large enough to justify it.
- [ ] Run withheld-glitch rediscovery campaigns only after their scenario and
  semantic oracle exist.
- [ ] Investigate complex memory corruption, wrong warps, multi-map setup
  chains, platform-specific rendering, and poorly sourced tricks only through
  explicit human selection.
- [ ] Importing a Skybook page must never auto-generate requirements, readiness
  state, observers, reproduction tasks, or an implied promise of support.

## Exhaustive game-state query toolbox

- [ ] Enumerate every player procedure, subprocedure, mode flag, timer,
  animation lane, combat state, equipment state, locomotion state, and form.
- [ ] Enumerate every live process group, relationship, collision primitive,
  attack/contact manifold, path, rail, spline, trigger, switch, event, quest,
  save, inventory, resource, heap, particle, audio, renderer, and UI state.
- [ ] Build whole-game static RARC/DZS/DZR/KCL/PLC inventories and spatial
  queries beyond the maps exercised by active objectives.
- [ ] Add generalized native query discovery, subscriptions, watch expressions,
  reflection/code generation, GraphQL-like APIs, or arbitrary expression
  evaluation.
- [ ] Add bounded address/layout diagnostics beyond a concrete reverse-
  engineering need while keeping semantic facts as portable proof authority.
- [ ] Build a complete camera/render/audio/effects observer catalog.

## Full determinism and fidelity program

- [ ] Audit every audio, movie, job, streaming, shader, renderer, host-I/O,
  alarm, thread, and asynchronous completion path across the whole game.
- [ ] Inventory and attribute every global, subsystem, actor-local, particle,
  audio, and JMath RNG stream and call site.
- [ ] Define core, route, extended, and full-checkpoint canonical state hashes
  covering the entire supported game state rather than active-objective facts.
- [ ] Establish exhaustive realtime/headful, unpaced, hidden-headful,
  null-renderer, emulator, and console parity corpora.
- [ ] Build a complete machine-readable fidelity matrix for native safety fixes,
  `AVOID_UB`, GC layout/address behavior, floating point, GX/cache traversal,
  and console-only quirks.
- [ ] Maintain emulator or console transfer cases not required by an active
  native-port objective.

## Persistent engine sessions and checkpoints

- [ ] Replace process-per-run execution with a persistent engine-session worker
  supporting load, reset, step, batch, cancel, capture, and health commands.
- [ ] Prove repeated soft reset restores all relevant managers, heaps, globals,
  threads, loaders, RNG, audio, input, and automation-owned state.
- [ ] Implement tiered checkpoints for route boundary, gameplay state, and full
  supported process state with relocation/fixup validation.
- [ ] Detect unsafe in-flight asynchronous work and refuse checkpoint capture or
  restore.
- [ ] Add checkpoint lineage, validation windows, memory accounting, eviction,
  compression, and corruption recovery.
- [ ] Add MCTS or other search that depends on validated checkpoint restoration.
- [ ] Pursue renderer/audio suppression, copy-on-write pages, batched stepping,
  or other throughput optimization only after profiling the active harness.
- [ ] OS process snapshots and forkservers remain won't-do unless portable
  reset/checkpoint approaches demonstrably fail.

## Advanced reinforcement learning

- [ ] Adopt Double-Q, CQL, IQL, prioritized replay, ensembles, dueling heads,
  distributional values, noisy exploration, or Rainbow components only after
  simple baselines fail under an equal-budget active objective.
- [ ] Add option-level goal conditioning, hindsight relabeling, online updates,
  replay generations, or policy hierarchies beyond the first proven baseline
  learning loop.
- [ ] Add recurrent critics only after a measured current-state aliasing case.
- [ ] Add DeepSets/attention only after fixed semantic actor slots fail on a
  sufficiently large content-disjoint corpus.
- [ ] Add graph encoders only after simpler pooling loses an equal-row
  comparison on an active objective.
- [ ] Add frozen native/accelerator inference, ONNX deployment, or per-tick
  inference after measuring that Rust/IPC batch placement is inadequate.
- [ ] Add transfer or multi-task learning only across explicitly compatible
  objective, action, observation, game-data, and fidelity identities.
- [ ] Require meaningful corpus size, action/state coverage, held-out boundary
  families, calibration, OOD diagnostics, and a win over structured/tree
  baselines before making neural performance claims.

## Model-based planning

- [ ] Learn local dynamics only after measuring prediction error for the active
  contacts, procedures, RNG branches, and actor interactions.
- [ ] Add Dyna mixtures only with real-rooted rollouts, strict uncertainty
  cutoffs, immutable lineage, and an explicit synthetic/real ratio cap.
- [ ] Add latent visual/world models only for observations unavailable through
  trustworthy semantic queries or for a concrete console-transfer case.
- [ ] Use learned values as search heuristics rather than promotion authority.

## Autonomous novelty and discovery

- [ ] Expand semantic novelty descriptors and archives beyond facts needed to
  distinguish winners for active objectives.
- [ ] Run open-ended campaigns for unseen procedures, contacts, transitions,
  event sequences, crashes, hangs, OOB routes, corruptions, or rare state
  combinations.
- [ ] Maintain symptom clusters, novelty rewards, discovery minimization,
  automated headful replay/video capture, and human classification queues.
- [ ] Feed labels into corpus metadata without rewriting objective or proof
  history.
- [ ] Do not treat spatial novelty or model surprise as success evidence.

## Experimental causal interventions

- [ ] Expand typed intervention tapes, parameter search, minimization, and
  control/treatment evidence beyond an explicitly approved mechanism study.
- [ ] Keep interventions compile-time disabled, runtime opt-in, phase-bounded,
  preconditioned, audited, and unmistakable in every artifact.
- [ ] Reproduce an intervention-discovered setup with ordinary controller input
  before promoting it from existence/mechanism evidence to normal proof.
- [ ] Arbitrary address writes remain outside ordinary builds and evidence.

## Full graphical workbench

- [ ] Add a general objective/query editor, schema browser, trace preview,
  run dashboard, archive/model visualization, and side-by-side candidate diff.
- [ ] Add arbitrary observation scrubbing, synchronized video/trace views,
  contact/path overlays, checkpoint controls, and campaign management.
- [ ] Expand route graph editing only when the CLI/operator loop demonstrates
  repeated review friction that a focused UI would remove.
- [ ] Keep one segment hierarchy and do not create algorithm-specific project
  trees for samples, models, milestones, or generated results.

## Distributed and multi-client execution

- [ ] Deterministic multi-client simulation, cross-client message schedules,
  barriers, delay/loss injection, and synchronized proof.
- [ ] Remote worker discovery, leases, retries, artifact transfer, heterogeneous
  capability scheduling, and Byzantine/corrupt-worker detection.
- [ ] Distributed corpora, object stores, manifests, garbage collection,
  dashboards, quotas, NUMA pinning, and cluster throughput optimization.
- [ ] These remain won't-do until a successful single-host harness produces a
  measured workload that cannot be handled locally.

## Broad performance and hardening programs

- [ ] Whole-game observer performance budgets, long-soak leak tests, exhaustive
  sanitizer matrices, broad protocol fuzzing, and crash corpus reduction beyond
  the active harness surfaces.
- [ ] Cross-platform release packaging and CI for unsupported hosts.
- [ ] Automated artifact migration across every historical schema rather than
  explicit version rejection or a targeted migration needed by retained data.
