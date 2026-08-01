# Route-learning framework

## Mission

Build an algorithm-agnostic learning and search framework that finds routes and
solutions substantially better than human play.

The framework receives a native starting state, generic observations, the legal
action surface, and an authoritative terminal predicate. It must explore, learn
which behavior produced useful outcomes, optimize native completion cost, and
emit an exactly replayable input graph. Q-learning is one candidate, not a
requirement; use whichever online, offline, model-based, hierarchical, or search
method works best at the sample sizes and throughput we can actually sustain.

The action hierarchy must include all three sources:

- primitive controller inputs;
- reusable parameterized tactics supplied by us;
- tactics discovered, evaluated, promoted, composed, and retired by the system.

A human route is optional experience and a benchmark. It is never the route
definition, a required seed, or a ceiling on the learned policy.

Ordon is the first proving ground. Its native load zone is the only success
authority. The known 125-tick human route is a weak baseline: 123 ticks is the
first proof that the framework can improve it, not the final optimization target.

## Work queue

These milestones are coupled, not a waterfall. Keep a thin end-to-end campaign
runnable, advance the earliest missing learning capability, and pull throughput
or architecture work forward when measurement shows that it blocks the next
useful iteration. Do not build a perfect harness around a learner that cannot
solve the task. Remove completed items; keep detailed history in commits and
sealed campaign artifacts.

### 1. Make experimentation fast enough to support learning

- Use existing campaign evidence to attribute native execution, observation,
  save-state capture/restore, replay, learning, IPC, scheduling, persistence, and
  idle time. Do not rerun merely to derive another metric already present in raw
  evidence.
- Remove the largest measured cost, retain a matched before/after comparison,
  and repeat in descending measured order.
- Measure whether save-state branching beats replay from an authority point;
  simplify or remove it wherever it does not.
- Eliminate worker starvation, redundant native work, unnecessary serialization,
  and central bottlenecks. Keep queues and memory bounded.

Exit: a standard two-worker campaign completes within ten minutes, two workers
materially outperform one on unique useful native transitions per second, and
retained save-state reuse has a measured positive return.

### 2. Make scratch exploration reach sparse terminals

- Allow trajectories long enough to solve the task accidentally. A human takes
  roughly four seconds; exploration must support much longer attempts and useful
  branching instead of being trapped by an arbitrary short decision horizon.
- Explore primitive inputs, applicable supplied tactics, tactic parameters,
  restarts, and branch points with explicit coverage/diversity accounting.
- Expose generic state needed to distinguish outcomes: motion history, velocity,
  orientation, camera, analog input, prompted-action availability, action
  duration, and the resulting state deltas.
- Preserve unsuccessful trajectories. Straight travel, velocity loss, collision,
  rolling, camera alignment, and detours must be learnable from transitions and
  eventual outcomes, not encoded as Ordon-specific rewards.

Exit: repeated held-out scratch campaigns reach the native load-zone predicate
without a demonstration, authored route, waypoint sequence, proxy terminal, or
route-specific exception.

### 3. Make experience improve future behavior

- Evaluate learning/search algorithms against the real transition volume,
  horizon, branching factor, and update latency; do not preserve Q-learning if a
  different method learns more effectively from the available samples.
- Learn from both successful and unsuccessful trajectories with sparse terminal
  value and authenticated native tick cost.
- Support online collection, off-policy replay, prioritized reuse, temporal
  credit assignment, continued exploration, and escape from local optima.
- Surface the complete legal action set at each decision, including prompted
  actions such as roll, jump, mount, lift, or future game-specific affordances.
- Demonstrate causality using learned, frozen-policy, and random-valid treatments
  drawn from the same comprehensive campaign design and resource budgets.

Exit: on held-out seeds, accumulated experience changes future choices and the
learned treatment reaches terminals more often, with fewer native samples, or at
lower tick cost than both controls. It must continue improving after its first
success.

### 4. Learn and compose tactics

- Represent primitives and tactics through one interruptible, parameterized
  option interface so the learner can choose either at every valid boundary.
- Supply reusable tactics we know are valuable without supplying a route. Initial
  examples include moving toward a chosen heading, one-frame direction plus
  camera lock, camera lock plus action, rolling, and state-conditioned analog
  steering.
- Mine useful sub-trajectories and repeated control structure from collected
  experience; propose tactics with learned initiation conditions, parameters,
  termination conditions, and expected outcomes.
- Evaluate proposed tactics against primitives and existing tactics, promote
  those that improve held-out search or policy performance, retire regressions,
  and allow tactics to compose other tactics.
- Keep primitive actions available so a bad abstraction cannot cap the policy.

Exit: ablations show that supplied tactics improve search without encoding an
Ordon route, at least one automatically discovered/promoted tactic improves
held-out performance, and composition can exceed the trajectories from which the
tactics were learned.

### 5. Beat the human route and keep optimizing

- Optionally ingest ordinary human play as off-policy replay, extracting useful
  states, transitions, action availability, and sub-trajectories without
  behavioral cloning becoming mandatory.
- Search beyond the demonstration, retain diverse competitive routes, and spend
  native samples where uncertainty or improvement potential is highest.
- Authenticate the terminal and native tick count, minimize the winning input
  graph, and cold-replay candidates from the named root state.

Checkpoint: an independently learned Ordon route reaches the native load zone in
123 ticks or less and cold-replays twice with identical inputs, state identities,
terminal proof, and tick count.

Exit: continued learning/search materially beats the best available human route
under the same native rules and a fixed optimization budget. A demonstration
ablation shows that human play may improve sample efficiency but is neither
required nor a policy ceiling.

### 6. Prove the framework is durable and general

- Apply the unchanged observation, action, learning, tactic, orchestration, and
  evaluation contracts to a second native route.
- Make fresh and resumed concurrent campaigns logically reproducible, with exact
  ownership for sampling, updates, model publication, checkpoints, and accepted
  experience.
- Keep models, replay, checkpoints, and manifests compact, bounded, binary,
  versioned, checksummed, atomic, and migration-tested.

Exit: the second route passes scratch discovery, learned improvement over
controls, tactic use, and exact cold replay without route-specific framework
changes; interruption and resume reproduce accepted-experience identities.

## Engineering rules

These apply while completing every queue item; they are not a later cleanup
phase.

- Campaigns collect one comprehensive reusable evidence stream. A run should
  answer many questions, including useful analyses not anticipated when it was
  launched. Rerun only for a changed treatment, genuine replication, or raw
  evidence that could not have been captured previously.
- Record observations, legal actions, choices, executions, transitions, replay
  admissions, updates, tactic decisions, terminals, timings, retries, rejections,
  duplicates, and censored work with stable identities.
- Throughput means unique accepted native experience per wall-clock second, not
  attempts, queued work, duplicate branches, or replayed frames.
- Refactor oversized and mixed-responsibility code along observation, execution,
  state, replay, learning, tactic, worker, persistence, and reporting boundaries.
  Enforce source-size and dependency gates so monoliths do not regrow.
- Own child processes directly. Cancellation, timeout, crash recovery, cleanup,
  progress, and resume must not scan for or affect unrelated processes.
- Use focused tests during iteration and broad suites at meaningful integration
  checkpoints. Do not substitute repeated testing for implementation progress.
- Benchmark gains count only when generic mechanisms, held-out evaluation, exact
  replay, and appropriate controls rule out route-specific gaming.
