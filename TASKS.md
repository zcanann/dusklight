# Deterministic TAS learning and optimization roadmap

This is the sole dependency-ordered roadmap for the learning framework.
Completed implementation history belongs in commits and benchmark reports, not
in this file. A checked implementation detail is not evidence that learning
works.

## Product outcome

Given an exact source boundary, an authored terminal predicate, and optionally a
human demonstration, one unattended campaign must be able to:

1. restore the exact source state without replaying unrelated history;
2. give a state-reactive policy complete controller authority for the episode;
3. explore through persistent native workers without rendering or host pacing;
4. retain successes, failures, and circuitous trajectories as learning evidence;
5. improve proposals across generations without defining the action space
   relative to an incumbent tape;
6. optionally hand a learned result to a separate short-horizon refinement
   stage;
7. expose progress and replayable candidates through one usable workbench; and
8. promote only an exact input tape that reproduces from ordinary cold boot.

The first benchmark is the checked 125-tick `ToOrdonSprings` segment. It is a
small, inexpensive framework test, not a reason to build Ordon-specific learning
machinery.

## Terms that must not be blurred

- **Residual optimization** edits an incumbent or searches a bounded
  incumbent-relative surface. It proves the optimizer and execution harness.
- **Demonstration-assisted learning** may use an authenticated human trajectory
  for replay, behavior cloning, or curriculum, but the online policy retains the
  complete legal action surface.
- **From-scratch discovery** receives a checkpoint, terminal predicate, generic
  observations, and legal actions. It receives no incumbent-relative
  coordinates, path progress, or privileged route.
- **Promotion** installs only the compact realized PAD tape after repeated exact
  cold replay. A model, critic, score, training loss, or simulated state is never
  outcome authority.

Passing one regime does not stand in for another.

## Non-negotiable contracts

- Gameplay advances only through the native deterministic execution boundary.
- Policies and optimizers may read authenticated observations and emit legal
  controller input; they may not write gameplay state.
- Every checkpoint, terminal, model, feature schema, action schema, request,
  episode shard, tape, and report is content-bound and fails closed on drift.
- The chosen PAD, consumed PAD, pre-input observation, resulting state, terminal
  evidence, and charged simulated ticks remain auditable for every admitted
  episode.
- Training, validation, and held-out partitions are lineage-safe.
- Negative controls receive the same data split and training budget as their
  corresponding treatment.
- Failed experiments remain valid evidence. Do not tune the verdict, metric, or
  budget after seeing the result.
- Generated candidates remain ephemeral until explicit promotion.

## Current gate status

| Gate | Status | What is actually established |
|---|---|---|
| 1. Native state-reactive execution | **Passed** | Frozen native inference, independent Rust reinference, realized ordinary tape, and model-free cold replay agree exactly. |
| 2. Continuous residual baseline | **Passed as infrastructure** | Random and CEM search recover the degraded q131 canary and behave credibly on q125. Neither has produced a deterministic sub-125 route. |
| 3. Autonomous learning loop | **Passed mechanically** | One unattended campaign completed three collect/train/freeze/native-execute/ingest generations with persistent workers, crash-safe resume, and cold-replayable outputs. |
| 4. Learning adds value | **Open** | The equal-budget 40-cell experiment is only partially executed. No learning treatment has yet earned a comparison verdict. |
| 5. Learning workbench | **Passed narrowly** | The existing Route Workbench can run, inspect, replay, cancel, resume, clean, and promote optimization campaigns. This does not describe the separate route-planner authoring UX. |
| 6. From-scratch discovery | **Open** | No deterministic route has been discovered from goal and generic world state alone. |

Authoritative committed evidence:

- Gate 1:
  `docs/glitch-hunting/benchmarks/frozen-policy-gate1-20260721.json`
- Gate 2 degraded-canary recovery:
  `docs/glitch-hunting/benchmarks/ordon-degraded-q131-recovery-20260722.json`
- Gate 3 checkpoint diagnosis:
  `routes/Glitch Exhibition/intro/benchmarks/ordon-q131-goal-learning-checkpoints.md`
- Negative controls:
  `routes/Glitch Exhibition/intro/benchmarks/ordon-q131-goal-reachability-negative-controls.md`
- Reverse curriculum:
  `routes/Glitch Exhibition/intro/benchmarks/ordon-q125-reverse-curriculum.md`
- Gate 4 protocol:
  `routes/Glitch Exhibition/intro/benchmarks/ordon-gate4-learning-value-comparison-v2.plan.json`

## Current empirical truth

### What works

- Exact state-reactive native policy execution and model-free cold replay.
- Persistent checkpoint workers, deterministic budgets, crash-safe journals, and
  independently replayed evidence.
- Random and CEM residual search over the complete permitted proposal surface.
- Demonstration-backed replay corpora, frozen critics and policies, native
  episode ingestion, reverse curricula, generic tactics, and quality-diversity
  archives.
- Held-out critic evaluation and information-removal controls.
- Explicit separation between proposal evidence and promotion authority.

### What failed

The authenticated q131 learning loop completed three generations and twelve
native policy rollouts, but all twelve missed the terminal. Every generation
collapsed to one parent state and one action trajectory. The policies emitted
many distinct PAD values and visited many state identities, so the failure is
not an unsupported-action fallback or a dead native runner.

Held-out critics remained calibrated, while shuffled-outcome and previous-PAD
controls were substantially worse. The current evidence therefore says:

> The corpus contains learnable state/outcome signal, but the deployed
> exploration and policy-population process did not convert it into successful
> control.

Do not claim that the learning framework plans routes until this changes under a
predeclared held-out comparison.

### Useful but limited success

The reverse curriculum reached the authenticated frame-0 frontier. Its first 64
root-frontier candidates contained 49 terminal successes across 23 successful
behavior classes. This establishes that the native curriculum/search machinery
can preserve diverse valid continuations. It is not from-scratch discovery, and
its best route remained 125 ticks.

## P0 — Complete Gate 4 without moving the goalposts

The sealed v2 experiment contains:

- two held-out checkpoints: degraded q131 and incumbent q125;
- four deterministic seeds;
- five equal-budget treatments; and
- 40 total cells with no promotion authority.

Treatments:

1. independent random residual search;
2. CEM residual optimization;
3. demonstration-assisted state-reactive learning;
4. from-scratch state-reactive learning; and
5. learned proposals followed by CEM residual refinement.

### Matrix progress

| Checkpoint | Random | CEM | Demo-assisted | From-scratch | Learned + CEM |
|---|---:|---:|---:|---:|---:|
| q131 | 4/4 sealed | 4/4 executed, 0/4 sealed | 0/4 | 0/4 | 0/4 |
| q125 | 0/4 | 0/4 | 0/4 | 0/4 | 0/4 |

Eight of 40 cells have executed; four are sealed as final cell evidence.

The completed q131 baseline executions currently show:

| Treatment | Successful episodes | Success rate | Best first hit across seeds |
|---|---:|---:|---:|
| Random residual | 2,700 / 4,096 | 65.92% | 125 ticks |
| CEM residual | 3,162 / 4,096 | 77.20% | 125 ticks |

These numbers are baseline observations, not a Gate 4 verdict.

### Collection

- [ ] Seal and independently validate the four completed q131 CEM cells.
- [ ] Execute, seal, and validate four q125 random cells.
- [ ] Execute, seal, and validate four q125 CEM cells.
- [ ] Execute, seal, and validate four q131 demonstration-assisted cells.
- [ ] Execute, seal, and validate four q125 demonstration-assisted cells.
- [ ] Execute, seal, and validate four q131 from-scratch cells.
- [ ] Execute, seal, and validate four q125 from-scratch cells.
- [ ] Execute, seal, and validate four q131 learned-then-CEM cells.
- [ ] Execute, seal, and validate four q125 learned-then-CEM cells.
- [ ] Reopen every cell from its sealed inputs and reject missing, duplicate,
  extra, drifted, truncated, or over-budget evidence.

### Verdict

- [ ] Aggregate successful-episode rate, best valid first-hit tick, and median
  first-success simulated-tick charge by treatment and held-out checkpoint.
- [ ] Require the learned-plus-CEM treatment to clear at least one predeclared
  improvement threshold against both random and CEM.
- [ ] Require the authenticated shuffled-outcome and action-only controls to lose
  the predeclared held-out critic comparison.
- [ ] Publish the result even if
  `learning_advantage_demonstrated` is false.
- [ ] Keep the complete comparison non-promoting; separately cold-prove any
  candidate considered for route installation.

**Gate 4 passes only when learning wins the sealed comparison. Completing the
matrix with a losing result completes the experiment, not the gate.**

## P1 — Correct learned control from measured failures

This work may proceed alongside matrix execution, but every change is a new
controlled treatment. Do not silently alter the sealed Gate 4 arms.

- [ ] Run the targeted exploration treatment that gives sibling rollouts unique,
  deterministic bounded exploration while holding critic architecture, action
  surface, terminal, horizon, and native budget fixed.
- [ ] Compare parent-state diversity, action-trajectory diversity,
  contact/state coverage, terminal coverage, and held-out success against the
  collapsed q131 campaign.
- [ ] If terminal coverage improves, test whether the same treatment improves
  the sealed Gate 4 learning arms before changing model architecture.
- [ ] Add complete actor-set, collision/geometry, surface, event/loading, and
  temporal/history observations through bounded native encoders with exact
  Rust/native parity.
- [ ] Repeat equal-budget representation ablations to determine which new
  channel improves held-out terminal metrics.
- [ ] Change capacity, recurrence, or value-learning algorithm only when a
  controlled representation/capacity comparison identifies that bottleneck.
- [ ] Preserve a diverse replay/archive population across spatial, contact,
  action-phase, event, actor-relative, and outcome dimensions.
- [ ] Combine learned setup, locomotion, interaction, and frame synchronization
  with a separately budgeted short-horizon refinement stage.

## P2 — Demonstrate from-scratch goal discovery

Gate 6 is a separate product claim and must not inherit credit from residual
search or reverse curriculum.

- [ ] Start from the Link-control checkpoint and authored terminal predicate
  without an incumbent tape, incumbent-relative residuals, path coordinates, or
  route-progress features.
- [ ] Supply generic world observations, the complete factorized action surface,
  episode-long native action authority, and a generous or adaptive horizon.
- [ ] Use checkpoint-backed curriculum, intrinsic coverage, hindsight goals, or
  other generic discovery mechanisms without embedding the known route.
- [ ] Preserve diverse spatial, contact, action-phase, event, actor-relative, and
  outcome states rather than one distance-minimizing frontier.
- [ ] Produce at least one authenticated terminal success and export its exact
  raw PAD tape.
- [ ] Cold-replay that tape from ordinary boot and require identical per-tick
  gameplay and terminal evidence.
- [ ] Repeat from multiple deterministic seeds and report failures as well as
  successes.

**Gate 6 passes when the system discovers and cold-proves a deterministic Ordon
Springs route from goal and generic world state alone. It need not beat 125
ticks.**

## P3 — Close only evidence-driven capability gaps

- [ ] Finish machine-readable stage/profile coverage for observation fields
  actually selected by an admitted learner.
- [ ] Extend actor-family state, RNG/timers, resource loading, interaction, or
  proof channels only when a neutral coverage audit or benchmark identifies a
  concrete missing signal.
- [ ] Maintain complete-set encoders where fixed-slot truncation loses measured
  held-out signal.
- [ ] Keep the bootable-world and complete-actor survey as background coverage;
  do not let it displace the active learning experiments.
- [ ] Demonstrate a deterministic sub-125 Ordon route or publish a bounded
  proposal-coverage diagnosis. Never describe 125 as optimal without proof.

## Workbench acceptance

The existing learning workbench remains acceptable only while it preserves this
short path:

1. select a segment and goal;
2. start with safe defaults;
3. see generations, ticks, successes, current best, proposal source, workers,
   and blockers;
4. inspect and replay completed candidates;
5. cancel or resume without orphaned workers;
6. explicitly promote through cold replay.

Any regression in that path is a framework bug. Broader graph authoring,
content-browser CRUD, and blueprint-like route composition belong to
`TASKS_ROUTE_PLANNER.md`.

## Overall completion

This roadmap is complete only when:

- Gate 4 demonstrates a real held-out learning advantage under the sealed
  protocol;
- Gate 6 produces a cold-proven route without incumbent-derived guidance;
- the selected learned behavior can be handed to refinement and promoted through
  the ordinary exact-input proof path; and
- the complete workflow is usable without assembling private CLI stages or
  editing generated request files.

Until then, the accurate description is:

> Deterministic learning and optimization infrastructure exists. Residual search
> works. Learned route planning and from-scratch discovery remain unproven.
