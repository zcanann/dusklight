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

The latest bounded Ordon check after correcting heading execution found a
468-tick route; two seeds produced terminal outcomes. It did not establish
improved learning. The human example takes 125 active ticks to
`ordon_spring_load_committed`. Beating it is an initial capability check, not
the framework's specification or performance ceiling. Experiment details live
in [the native evaluation notes](docs/glitch-hunting/learning-evaluations/2026-09-06-heading.md).

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
- [ ] Make checkpoint branching and reuse of experience effective parts of
  exploration. Compare alternatives, revisit useful states, learn from every
  valid outcome, and allow enough exploration to escape local optima.
- [ ] Discover, evaluate, and reuse tactics that improve subsequent searches.
  Develop transfer beyond replaying exact recordings, and let learned tactics
  compete with supplied tactics and simpler actions.
- [ ] Use ordinary suboptimal human examples as optional learning experience.
  Demonstrate their contribution without requiring imitation or imposing a
  ceiling on the learned solution.

## Demonstrate usefulness

- [ ] Show repeatable improvement in complete native routes over simple search
  and frozen-learning controls under comparable budgets. Measure success rate,
  route quality, and time to useful results; distinguish independent discovery
  from shared or inherited experience. Verify claimed routes by cold replay.
- [ ] Find substantially better-than-human routes and apply the framework to
  additional goals. Use these results to expose limitations in the learner,
  representation, or actions, without baking benchmark solutions into them.
- [ ] Make iteration fast enough to be useful. Track where execution, restore,
  fitting, scheduling, persistence, and communication consume the budget; fix
  measured bottlenecks and evaluate parallelism by useful learning per wall time.

## Engineering foundations

- [ ] Make the active execution and learning path easy to follow and test.
  Separate responsibilities, organize related behavior into folders, and remove
  redundant paths. Production files over roughly 1,000 lines need decomposition
  or a concrete single-responsibility justification.
- [ ] Harden checkpoint ownership, recovery, experience admission, and worker
  orchestration so failures cannot silently corrupt experiments. Keep durable
  learning data bounded, binary, checksummed, and atomically written; reserve
  JSON for small human-facing summaries, manifests, and explicit diagnostics.
- [ ] Provide concise introspection that explains what was tried, what was
  learned, what drove choices, and where time went. Make failure causes visible
  without requiring a forensic reading of implementation details.

## How to use this list

- These are capability outcomes, not a fixed algorithm or mandatory sequence.
  Pick the next bounded milestone by its expected contribution to useful
  learning; address engineering foundations alongside the behavior they support.
- Keep experiment details and historical results outside this list. Each
  experiment needs a question, a cost limit, and a decision its result informs.
  Prefer checks that answer several related questions when practical.
- When work stops improving understanding or capability, reassess the approach.
  Replace or retire unhelpful subtasks instead of adding increasingly narrow
  acceptance gates. Remove completed work; tests alone do not prove usefulness.
- Use at most two owned build/native workers. Manage only child processes
  started by this session; never stop unrelated processes.
- Commit and push every natural milestone; keep the workspace clean.
