# Generic in-game goal learner

## Goal

Build a learning framework that discovers substantially better-than-human
routes and creative solutions in useful wall time. It should use supplied
tactics, discover and promote its own, and work with or without human examples.
Choose algorithms for the available experience and execution budget; Q-learning
is an option, not a requirement.

Success means reaching real game goals efficiently. Intermediate signals help
learning, but improvements to a proxy metric do not establish better routes.

## Starting point

Checkpoint restoration, transition graphs, replay, and terminal verification
provide useful foundations. The active learner still mixes several decision
mechanisms, loses some meaningful state distinctions, and exposes a narrower
action space than the surrounding interfaces suggest. Useful general learning
has not yet been demonstrated.

Beating a short human route is an initial capability check, not the framework's
specification or performance ceiling. Current results and experiment details
belong in [the evaluation notes](docs/glitch-hunting/learning-evaluations/),
not in an expanding set of benchmark-specific tasks.

## Learning and exploration

- [ ] Establish a simple, coherent learning loop whose decisions improve from
  experience. Make delayed outcomes influence earlier choices, including moves
  that look locally unhelpful. Consolidate competing models, gates, and fallback
  rules where they obscure or impede this behavior.
- [ ] Give the active learner observations that preserve meaningful differences
  between states, goals, and available actions. Check what actually reaches the
  model; retained raw data and broad schemas alone are insufficient.
- [ ] Provide an expressive, composable action space with usable control over
  direction, timing, duration, and contextual actions. Supplied tactics should
  accelerate discovery without becoming the boundary of possible solutions.
- [ ] Use save-state branching and accumulated experience throughout exploration.
  Try alternatives from useful states without repeatedly paying for their
  prefixes, retain what those attempts teach, and allow enough exploration to
  discover longer solutions before optimizing them.
- [ ] Discover, evaluate, and reuse tactics that improve subsequent searches.
  Develop transfer beyond replaying exact recordings, and let learned tactics
  compete with supplied tactics and simpler actions.
- [ ] Learn from optional human examples without requiring imitation or imposing
  a ceiling on the solution. Keep discovery without a baseline a first-class path.

## Demonstrate usefulness

- [ ] Demonstrate useful learning in the real game: repeatable discovery,
  substantially better-than-human routes, and creative solutions across goals.
  Compare against simple alternatives under comparable budgets to establish
  whether learning helps. Verify claimed routes from the original starting
  state; distinguish independent discovery from reused experience. Do not bake
  benchmark solutions into the learner or its success criteria.
- [ ] Make iteration fast enough to be useful. Track where execution, restore,
  fitting, scheduling, persistence, and communication consume the budget; fix
  measured bottlenecks and evaluate parallelism by useful learning per wall time.

## Engineering foundations

- [ ] Make the active execution and learning path easy to follow and test.
  Separate responsibilities, organize related behavior into folders, and remove
  redundant paths. Break up oversized modules around clear responsibilities
  rather than letting campaign-specific logic accumulate in central files.
- [ ] Harden checkpoint ownership, recovery, experience admission, and worker
  orchestration so failures cannot silently corrupt experiments. Keep durable
  learning data efficient, bounded, and recoverable, with binary serialization
  for bulk data. Surface failures instead of silently degrading the experiment.
- [ ] Provide concise introspection that explains what was tried, what was
  learned, what drove choices, and where time went. Make failure causes visible
  without requiring a forensic reading of implementation details.

## How to use this list

- These are capability outcomes, not a fixed algorithm or mandatory sequence.
  Pick the next bounded milestone by its expected contribution to useful
  learning; address engineering foundations alongside the behavior they support.
- Start with the simplest coherent learner, informative observations, and useful
  actions. Add complexity only to address an observed limitation; removing a
  mechanism is a valid improvement. Neither a particular algorithm nor a growing
  collection of heuristics is the deliverable.
- Keep experiment details and historical results outside this list. Each
  experiment needs a question, a cost limit, and a decision its result informs.
  Prefer checks that answer several related questions when practical.
- When work stops improving understanding or capability, reassess the approach.
  Replace or retire unhelpful subtasks instead of adding increasingly narrow
  acceptance gates. Remove completed work; tests alone do not prove usefulness.
- Finish milestones with a concrete capability change or a decision about what
  to change next. Do not turn one unresolved benchmark into an indefinite run
  of tuning and tests. If a task needs a design decision, state it and move to
  independent work; implementation difficulty alone is not a blocker.
- Use at most two owned build/native workers. Manage only child processes
  started by this session; never stop unrelated processes.
- Commit and push every natural milestone; keep the workspace clean.
