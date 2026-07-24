# Active task: make an agent learn a route

This is the sole dependency-ordered roadmap for the learning framework.
Implementation history belongs in Git and benchmark reports. This file contains
only the current product target and unfinished work.

## The product in one paragraph

Give an agent:

- an exact starting checkpoint;
- an authored binary goal;
- a typed catalog of observable facts and derived measurements; and
- a library of applicable actions or multi-tick tactics.

The agent chooses tactics, observes what changed, learns which choices lead to
valuable future states, branches again from retained checkpoints, and eventually
reaches the goal. A tactic emits ordinary controller input underneath. A
successful tactic chain becomes an exact PAD tape and must reproduce from cold
boot before promotion.

A human route may optionally seed experience. It must not define the learner's
action space, state coordinates, or only path to success.

## First proof

The first proof starts from the authenticated Ordon Springs Link-control
checkpoint and uses the existing `ordon_spring_load_committed` terminal.

The agent receives:

- generic world, player, actor, surface, event, and history facts;
- generic goal-relative and change-relative measurements;
- the complete applicable tactic catalog; and
- enough native simulation budget to learn from repeated branching.

It does **not** receive:

- the q125 tape or another human demonstration;
- incumbent-relative residuals;
- authored route coordinates or route-progress indices;
- a hidden sequence of preferred tactics; or
- gameplay-state writes.

The proof passes when the learned greedy tactic policy reaches the terminal,
exports the exact realized PAD sequence, and that tape reproduces the same
per-tick gameplay and terminal result from ordinary cold boot.

Route speed does not matter for this first proof.

## Current truth

| Capability | State |
|---|---|
| Deterministic checkpoints and persistent native workers | Working |
| Learner-facing state | Working: each authenticated `FactSnapshot` uses the shared typed query/measurement registry and carries the full action universe, applicability mask, parameters, duration, schema digests, and a compact bounded infodump derived from that same state |
| Authored terminal predicates | Working |
| Reactive world-space movement | Working: seek coordinate, actor, path point, opening, plane, heading, offset, and distance |
| Static motion paths | Working: waypoint, rail, Catmull-Rom spline, and cubic Bézier |
| Controller composition | Working separately for concurrent movement/camera/button/clamp layers and sequential static search actions |
| Game-specific and generic bounded tactics | Working, including exact PAD/query capture and experience-mined initiation/termination predicates |
| Semi-Markov option values | Working: duration-aware fitted Q iteration, typed option catalogs, ranking, and deterministic selected-option execution |
| Common executable tactic catalog | Working: existing game tactics, native generic tactics, motion paths, and DUSKCTRL programs share one finite runtime catalog; deterministic applicability enumeration returns concrete parameterized entries and bounded blueprints whose current start path is applicable, permits an explicit empty dead-end set, and binds the exact learner-visible choice schema by digest |
| Replay corpora, critics, policies, and checkpoint archives | Working as separate components |
| Exact realized tape and cold-replay proof | Working: the first learned route's 932-frame process-boot tape reproduced the same terminal and identical semantic gameplay state at all 932 recorded boundaries across two cold runs |
| Blueprint composition asset model | Working: canonical bounded assets reference executable catalog entries through `Invoke`, `Sequence`, `Layer`, `Conditional`, `Until`, and `Fallback`; static sequences compile into one exact tape with contiguous per-option execution records, layers compile through DUSKCTRL ownership rules, and ambiguous writers, unbounded control flow, unavailable conditions, invalid catalog plans, and any loss of exact PAD fail closed |
| Live online option-Q campaign | Working: authenticated tactic boundaries feed duration-aware replay, refit, ranking, reward shaping, hindsight, checkpoint/resume, final-result export, and an exact observed-state greedy table that prevents sparse fitted-Q extrapolation from overriding known successful decisions |
| Automatic checkpoint branching driven by learned tactic value | Working: campaigns retain replayable quality-diversity frontiers, sample root plus frontier branches, reject detached restores, detect collapse/cycles/connectivity loss, and project checkpoint-keyed state/tactic graphs |
| Learning workbench | Working: `Learn route` launches the tactic-Q campaign directly from the selected authenticated start and authored goal with bounded defaults and no generated request or demonstration editing; the primary projection carries only a compact latest-decision summary, while authenticated on-demand inspection loads its before/after facts, named measurements, applicable tactic/Q values, reward components, and resulting state change; a compact spatial graph shows checkpoint states, tactic edges, retained frontier cells, the current state, and terminals without loading duplicated route tapes; every projected edge is inspectable and its exact candidate path replays on demand through an ordinary process-boot launch; pause seals a content-addressed campaign checkpoint, resume cold-launches a fresh append-only native-worker attempt and reuses sealed seeds, cancel preserves authenticated evidence behind a durable marker, and cleanup remains unavailable until worker shutdown |
| Blueprint-like user-authored tactic assets | Working end to end: the content browser separates 136 immutable Library tactics from independently serialized Workspace assets; sequence blueprints support stale-safe create, edit, rename, duplicate, and recoverable delete; advanced typed blueprints remain visible without lossy editing; and the exact authored set is loaded into live learning and authenticated by the route action-schema digest |
| A route learned from goal, facts, and tactics | Working: seed 181081 reached the terminal after 70 no-demonstration decisions; the frozen policy then reached it in 13 greedy decisions and cold-proved tape `872f7f...`. See `docs/glitch-hunting/benchmarks/ordon-tactic-q-first-proof-20260724.json` |

## Architectural reset

### Tactics are the learning actions

Do not begin by asking a learner to rediscover controller mechanics every frame.
The learned action space consists of bounded options such as:

- wait;
- face a target or direction;
- move toward or away from a target;
- move along a heading;
- roll;
- interact;
- hold or pulse a button;
- continue until a fact query changes; and
- execute a user-authored blueprint-like composition of other tactics.

Every tactic implements one contract:

```text
identity + version
typed parameter schema
applicability query
bounded execution policy
success/stop query
maximum duration
emitted PAD frames
resulting fact snapshot
```

Built-in native tactics and user-authored tactics use the same contract. The
learner sees only currently applicable, concretely parameterized choices.
Existing `GameTacticPlan`, `NativeGenericTacticPlan`, `MotionPathPlan`, and
reactive-controller programs adapt into this contract without losing their
current typed serialization or exact execution behavior.

### Facts are typed; infodumps are projections

The learner consumes one stable typed view over existing observation artifacts,
not prose:

- stage, room, layer, procedure, and loading state;
- position, velocity, facing, animation/action phase, and grounded state;
- collision, contact, surface, ledge, and correction state;
- nearby actor identity, family, state, and relative transform;
- event, flag, inventory, resource, and interaction state;
- recent tactic, recent PAD, recent state changes, and elapsed ticks; and
- terminal-related entities and measurements exposed by the goal context.

A human-readable infodump is generated from that same snapshot for inspection.
Tactics, goals, UI panels, and the learner query the same fact/measurement
registry instead of maintaining private representations.

### Binary goal, measurable progress

The terminal predicate remains the only authority for success. Learning may use:

- terminal reward;
- elapsed-tick cost;
- changes in goal-relative distance, angle, state, or event measurements;
- new events, interactions, contacts, surfaces, rooms, and actor relationships;
- novelty and frontier coverage; and
- hindsight goals derived from states actually reached.

Prefer potential-based shaping:

```text
reward = terminal_reward + gamma * potential(next) - potential(current)
         - tick_cost + novelty
```

Progress measurements guide exploration; they never declare the route complete.

### Q-learning operates over tactic transitions

One experience row is:

```text
state facts
chosen tactic + parameters
accumulated reward
duration in ticks
next-state facts
terminal verdict
checkpoint and exact PAD range
```

This is a semi-Markov decision process because tactics last multiple ticks.
Update the long-term value of a tactic using the duration-discounted value of
the next applicable tactic. A small fitted Q model is sufficient for the first
proof; do not add ensembles, recurrence, or a novel learning algorithm without a
measured need.

Exploration begins with epsilon-greedy or uncertainty-aware tactic choice.
Retained checkpoints allow the agent to branch repeatedly from useful or novel
states instead of replaying the entire route for every decision.

## P2 — Add demonstrations and refinement without corrupting the model

- [ ] Import an optional human tape as replay transitions or tactic examples
  through the same state/tactic interface used by autonomous experience.
- [ ] Prove that removing the demonstration does not remove any action,
  observation, measurement, checkpoint, or terminal capability.
- [ ] Compare cold-start and demonstration-seeded learning by time to first
  terminal success.
- [ ] Hand a learned successful tape to a separately budgeted short-horizon
  continuous/discrete refinement stage.
- [ ] Promote only the final exact tape after ordinary cold replay.

## P3 — Optimize throughput only when measured

- [ ] Measure useful tactic decisions, native ticks, and complete learning
  episodes per second on the actual tactic-Q loop.
- [ ] Break wall time into simulation, checkpoint restore, fact extraction,
  tactic execution, model update, compression, persistence, and UI projection.
- [ ] Benchmark worker counts appropriate to the current 24-thread host before
  changing emulator or evidence code.
- [ ] Increase batching and worker utilization when the learner is starved for
  diverse transitions.
- [ ] Optimize implementation code only when profiling identifies a phase that
  materially limits a meaningful learning experiment.

Throughput is successful when an experiment can collect enough diverse tactic
transitions to improve behavior promptly. A larger number of identical failed
trajectories is not useful throughput.

## P4 — Validate the claim after competence exists

The existing Gate 4 comparison protocol and completed baseline cells are
retained. The tactic-level learner and its authoring workflow now work, so
validation can proceed:

- [ ] Define a smaller sealed comparison that uses the actual tactic-Q learner,
  not the abandoned per-tick policy as a proxy.
- [ ] Compare it against random tactic selection and a non-learning tactic
  search under equal native-tick budgets.
- [ ] Repeat across multiple seeds and at least one held-out start state.
- [ ] Publish success rate, time to first success, best route, and useful state/
  tactic coverage even if learning loses.
- [ ] Run the larger 40-cell protocol only if its additional treatments answer a
  remaining product question.

Scientific validation confirms a working learner. It is not a prerequisite for
building one.

## Explicitly removed from the critical path

- Per-frame analog policy learning as the first agent abstraction.
- Further architecture or negative-control sweeps before P0.
- Completing the old 40-cell matrix before a tactic learner succeeds.
- Treating residual optimization as route discovery.
- Making every transient rollout, model, and replay update a sealed publication.
- Broad world/actor survey work not selected by the active learner.
- Claiming that the 125-tick human route is optimal.

## Overall completion

The framework is a route learner when a user can select a start and goal, provide
or create tactics, press `Learn route`, watch the agent build understandable
state/tactic knowledge, and receive a successful exact tape that reproduces from
cold boot.

Until then, the accurate description is:

> We have deterministic execution, optimization, and proof infrastructure. The
> simple tactic-level learning product is not built yet.
