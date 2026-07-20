# Project scope and objective selection

Dusklight's near-term success criterion is narrower than the eventual glitch-
hunting vision: improve the existing Link-control-to-Ordon-Springs tape faster
than a human can improve it frame by frame, then cold-replay the result exactly.

## Core scope

- Reuse a live game process and an exact Link-control checkpoint for cheap
  suffix experiments.
- Run observation and policy decisions at the native pre-input tick boundary.
- Expose only the read-only movement and spatial facts required by this route.
- Compare raw mutation, structured tactics, continuous search, and learned
  proposals under the same useful-simulation budget.
- Retain the realized input tape and require independent cold replay before
  promotion.

## Objective-driven expansion

The Ordon benchmark is the gate for new harness work. Add an observation,
action, algorithm, or operational tool only when a measured limitation of that
benchmark requires it. Existing experimental scaffolding does not create an
obligation to expand it.

## Active roadmap

`TASKS.md` is the sole roadmap. Its completion gate is a cold-proven, multi-tick
machine improvement over the retained human Ordon tape.

Do not add arbitrary game-state writes to ordinary agent runs. Do not let a
learner, novelty score, route topology, checkpoint, intervention, or visual
impression bypass semantic objective evidence and cold absolute-tape replay.
