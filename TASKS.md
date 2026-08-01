# Learning framework tasks

## Goal

Build a route-independent native learner that can discover a reachable terminal,
learn which behavior caused progress, and improve a route quickly enough to be
useful. Ordon is the first acceptance test, not the architecture.

Success means all of the following:

- Scratch learning beats budget-matched frozen-policy and random-valid controls
  on held-out seeds.
- The learned route reaches the native load-zone predicate in 123 ticks or less
  and cold-replays twice with identical inputs, states, proof, and tick count.
- The standard two-worker learning/control experiment finishes within ten
  minutes on the named Windows host.
- Adding a second worker materially increases unique useful native experience
  per wall-clock second.
- The same framework works on a second route without route-specific code,
  rewards, actions, or observations.

## Queue

Work top to bottom. A task is complete only when its exit condition is supported
by sealed, reproducible evidence. Remove completed tasks; history belongs in Git
and experiment artifacts.

### 1. Establish one trustworthy baseline

- Run one build-matched learned/frozen/random experiment with predeclared seeds,
  native-experience budgets, and resource limits.
- Trace every observation, legal action, choice, execution, transition, replay
  decision, update, publication, retry, rejection, duplicate, and terminal.
- Repeat the same workload three times with one worker and three times with two.

Exit: all V6 audits and semantically bound completion artifacts validate; learned
updates cause authenticated same-state action changes; controls receive no update
benefit; and outcome, variance, throughput, utilization, and lost work reconcile.

### 2. Make useful exploration fast

- Attribute wall time and worker capacity to observation, native execution,
  save-state capture/restore, replay, learning, IPC, queues, persistence, and
  reporting.
- Fix the largest measured loss, then repeat the matched baseline. Continue in
  descending measured order instead of optimizing assumed bottlenecks.
- Keep save states only where capture, validation, and restore beat replay from
  an authority point.
- Provide exact owned-process cancellation, cleanup, crash recovery, progress,
  and resume without orphaning workers or double-counting experience.

Exit: the standard experiment finishes within ten minutes, two workers provide a
material useful-throughput gain over one, queues remain bounded, and retained
save-state reuse has a measured positive return.

### 3. Discover a sparse terminal from scratch

- Give primitive-control exploration enough horizon, coverage, and diversity to
  reach a human-reachable load zone without a demonstration.
- Expose versioned generic observations: motion history, velocity, orientation,
  camera, analog input, prompted action availability, and transition outcomes.
- Learn from native transitions and the terminal predicate; do not add authored
  waypoints, routes, straight-line/wall/roll rewards, blessed sequences, proxy
  terminals, or Ordon exceptions.

Exit: repeated held-out scratch runs reach the native load-zone predicate and
beat both controls under identical native-experience and resource budgets.

### 4. Learn and optimize behavior

- Propagate sparse terminal value through collected experience while continuing
  exploration after the first success.
- Escape local optima and optimize authenticated native tick cost.
- Learn, parameterize, compose, promote, and retire reusable multi-frame tactics
  while keeping primitive controls available. This includes discovering useful
  roll, camera-lock, and prompted-action behavior from outcomes rather than
  encoding an Ordon solution.
- Treat human replay only as optional off-policy experience and measure its
  effect separately.

Exit: learned held-out performance beats both controls, reaches 123 ticks or
less, cold-replays twice exactly, and an ablation shows demonstrations are neither
required nor a ceiling on the learned policy.

### 5. Harden the framework and prove it general

- Split observation, action, execution, replay, learning, publication,
  persistence, orchestration, and reporting into owned, testable components;
  enforce dependency and source-size limits that prevent new monoliths.
- Make concurrent ordering, checkpoint publication, interruption, and resume
  logically reproducible.
- Keep durable models, replay, checkpoints, and manifests compact, bounded,
  binary, versioned, checksummed, atomic, and migration-tested.
- Run the unchanged framework on a second native route.

Exit: focused contract/fault/replay tests cover every boundary; fresh and resumed
runs reproduce evidence identities; invalid durable state fails closed; and the
second route passes scratch discovery, controlled improvement, and cold replay.

## Experiment rules

- Only the native terminal predicate proves success.
- Compare treatments by useful native experience and fixed resources, never by
  convenient decision counts or favorable seeds.
- Useful throughput counts unique accepted native transitions, not attempts,
  duplicate branches, replayed frames, or queued work.
- Every experiment states one falsifiable question and seals the build, inputs,
  seeds, budgets, treatment, failures, and complete accounting.
- Benchmark improvement counts only when controls, ablations, and exact replay
  show that a route-independent mechanism caused it.
