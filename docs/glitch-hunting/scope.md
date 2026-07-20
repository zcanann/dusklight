# Project scope and objective selection

Dusklight's near-term success criterion is a trustworthy automation harness for
controlling agents, expressing semantic objectives, querying game state,
collecting learning evidence, and narrowing proposals into cold-replayable
winners. The harness should first prove itself on cheap ordinary gameplay—not
on an ambitious catalog of known glitches.

## Core scope

- Establish process and stage boot through a declared scenario fixture.
- Give scripted, random, structured, and learned agents one bounded execution
  contract.
- Expose typed, read-only facts required by a checked-in objective.
- Evaluate a semantic objective independently of learner reward or score.
- Retain the exact realized input tape, episode evidence, and complete identity.
- Compare proposal sources fairly, minimize finalists, and require independent
  cold replay before promotion.

The first conformance objectives should be intentionally mundane and easy to
inspect:

1. boot into a map and establish the declared ready state;
2. walk into a bounded target region;
3. approach and talk to one exact placed NPC; and
4. approach and pick up one exact carryable object.

Each objective needs a negative control. These cases exercise boot, input,
queries, actor identity, interaction state, predicates, traces, episode storage,
search/learning integration, replay, and diagnosis without making glitch setup
complexity part of the infrastructure test.

## Objective-driven expansion

An authored objective is the gate for new harness work. Add only the observation
families, actions, controller features, and operational tooling needed to run
and diagnose that objective. Missing capability should produce an explicit
unsupported result, not a guessed value or a speculative whole-game subsystem.

Glitches remain useful later because they stress unusual state and timing, but
they are not inherently better harness tests than ordinary interactions. The
revision-pinned Skybook manifest remains inert reference material until a human
chooses a page after the basic objective suite is reliable.

## Active roadmap

`TASKS.md` is the sole roadmap. Its current completion gate is deliberately
narrow: persistent native episodes, one exact Link-control checkpoint, generic
tactics, causal learning, and a cold-proven machine improvement over the human
Ordon route. Larger glitch, UI, multiclient, distributed, and general savestate
programs are not active work merely because experimental scaffolding exists.

Do not add arbitrary game-state writes to ordinary agent runs. Do not let a
learner, novelty score, route topology, checkpoint, intervention, or visual
impression bypass semantic objective evidence and cold absolute-tape replay.
