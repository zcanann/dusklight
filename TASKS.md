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
| Typed native observations and complete PAD authority | Working, though not yet exposed through one minimal learner-facing fact API |
| Authored terminal predicates | Working |
| Generic bounded tactic execution | Exists as infrastructure, not as the central learning action space |
| Replay corpora, critics, policies, and checkpoint archives | Working as separate components |
| Exact realized tape and cold-replay proof | Working |
| Online tactic-level Q learner | Missing |
| Automatic checkpoint branching driven by learned tactic value | Missing |
| Blueprint-like user-authored tactic assets | Missing |
| A route learned from goal, facts, and tactics | Not demonstrated |

The previous q131 campaign was not this product. It trained a per-tick policy,
ran only twelve native online rollouts, and collapsed to one trajectory per
generation. The 40-cell comparison protocol measures that complicated learner;
it is not the current critical path.

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

### Facts are typed; infodumps are projections

The learner consumes a stable typed snapshot, not prose:

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

## P0 — Build the minimum competent agent

Work in this order.

### 1. Unify the tactic runtime

- [ ] Define one serialized, versioned `Tactic` contract for identity,
  parameters, applicability, execution, stopping, duration, queried facts, and
  emitted PAD.
- [ ] Wrap the existing native generic tactics behind that contract.
- [ ] Implement the minimum built-in Ordon-independent tactic set: wait, face,
  directed movement, target-relative movement, roll, interact, and bounded
  button hold/pulse.
- [ ] Add a blueprint-like composite tactic whose nodes invoke tactics, branch on
  typed fact queries, and expose typed parameters.
- [ ] Enumerate a finite set of concrete applicable tactic choices from the
  current state, including bounded parameter candidates.
- [ ] Record every tactic execution as an exact PAD range plus before/after fact
  snapshots and stop reason.

### 2. Expose one fact and measurement surface

- [ ] Define a versioned `FactSnapshot` containing the generic player, world,
  actor, surface, event, interaction, and short-history state needed above.
- [ ] Define a typed fact-query and derived-measure registry shared by goals,
  tactics, the learner, and the UI.
- [ ] Expose generic relational measures: distance, angle, relative velocity,
  contact/surface relation, state change, event change, and elapsed ticks.
- [ ] Expose the complete applicable-tactic mask and concrete tactic parameters
  alongside every snapshot.
- [ ] Generate a readable infodump from `FactSnapshot` without introducing
  additional hidden state.
- [ ] Prove native execution and Rust decoding produce identical snapshots,
  queries, measures, tactic choices, and emitted PAD for one multi-tactic trace.

### 3. Implement the small Q loop

- [ ] Define the tactic-transition replay row shown above, including duration,
  terminal, checkpoint, and realized PAD identities.
- [ ] Implement a small fitted `Q(state, tactic, parameters)` model over typed
  facts and the finite applicable action set.
- [ ] Implement duration-aware Q targets, replay sampling, target refresh, and
  epsilon-greedy tactic selection.
- [ ] Implement configurable potential-based shaping, tick cost, novelty, and
  terminal reward while keeping terminal authority separate.
- [ ] Add hindsight rows for useful states actually reached, without fabricating
  gameplay or terminal outcomes.
- [ ] Make one unattended loop perform:
  `restore -> observe -> choose tactic -> execute -> retain transition -> update
  Q -> choose frontier checkpoint`.
- [ ] Persist only enough crash-safe state to resume the learner and authenticate
  a final result. Do not content-seal every transient training object in the hot
  path.

### 4. Branch from useful states

- [ ] Retain checkpoints for terminal successes, novel fact/relationship states,
  meaningful event changes, and high-value frontiers.
- [ ] Bound the archive while preserving diversity across position, contact,
  surface, action phase, actor relationship, event state, and outcome.
- [ ] Sample starts from both the root and retained frontiers so the agent learns
  complete routes rather than only terminal-local continuations.
- [ ] Detect zero-diversity tactic selection, repeated identical trajectories,
  no-progress loops, and a frontier that loses root connectivity.
- [ ] Show the learned state/tactic/checkpoint graph so a human can see what the
  agent believes leads toward the goal.

### 5. Prove the simple learner

- [ ] First prove the Q implementation on a deterministic fixture requiring a
  nontrivial multi-tactic sequence and delayed reward.
- [ ] Run the no-demonstration Ordon campaign from the authenticated Link-control
  checkpoint with a fresh model and multiple exploration seeds.
- [ ] Show that Q values, tactic selection, frontier coverage, and terminal
  success improve during the run; training loss alone is irrelevant.
- [ ] Freeze the greedy tactic policy and execute it from the root checkpoint
  without exploration.
- [ ] Export its exact realized PAD tape.
- [ ] Cold-replay that tape from ordinary boot and require identical per-tick
  gameplay and terminal evidence.

**P0 is complete only when the agent learns and cold-proves the route.**

## P1 — Make the learning loop usable

- [ ] Add one `Learn route` action for a selected start and goal with safe
  defaults and no generated request-file editing.
- [ ] Show current facts, derived measurements, applicable tactics, chosen
  tactic, Q values, reward components, and the resulting state change.
- [ ] Show the retained frontier and learned state/tactic graph without flooding
  the screen with per-tick evidence.
- [ ] Allow inspection and replay of any tactic edge or complete candidate path.
- [ ] Add content-browser CRUD for user-authored blueprint tactics while keeping
  built-in tactics visible and read-only.
- [ ] Support pause, resume, cancel, and cleanup without orphaned workers or
  losing the best authenticated route.
- [ ] Keep detailed traces and proof artifacts on demand rather than permanently
  occupying the primary workspace.

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
retained, but further matrix execution is parked until P0 succeeds.

After one tactic-level learner works:

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
