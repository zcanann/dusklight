# Glitch-hunting platform

This directory defines the fork-specific work needed to turn Dusklight into a
deterministic, high-throughput platform for reproducing and discovering
Twilight Princess glitches.

The platform should make a run useful in three different ways:

1. replay the same raw controller input exactly;
2. execute it as quickly as possible without changing logical time; and
3. promote an interesting headless run into a visible client at any chosen
   tick.

The guiding rule is that every result is an artifact, not an anecdote. A found
glitch should carry its scenario, input tape, build identity, observations,
state hashes, and the reason it was classified as interesting.

## Core decisions

- Rust owns orchestration, scheduling, corpus management, and the CLI.
- C++ inside Dusklight owns each simulation tick, input application,
  observations, checkpoints, and game-specific instrumentation.
- Workers are long-lived native processes. We do not start a process or cross
  an FFI/IPC boundary for every frame.
- Headless means presentation-free and unpaced, not a different update rate.
- Raw input tapes are the replay authority. Static splines compile directly to
  tapes; observation-feedback controllers run against read-only snapshots and
  emit a realized tape for exact replay and promotion.
- Automation may write the exclusive virtual PAD and automation-owned output
  only. Gameplay observations are read-only, and an inactive controller takes
  a cold path without scanning actors or touching PAD ownership.
- Fidelity is explicit. Native safety fixes, original bugs, relative heap
  layout, absolute GameCube addresses, and console rendering are different
  capabilities and must not be conflated.
- Python may be useful for offline notebooks, plots, and one-off corpus
  analysis, but it is not part of the execution hot path.

## Documents

- [Implementation status](status.md) records working commands, tests, and
  current fidelity/validation limits.
- [Testing and visual TAS playback](testing.md) documents the VS Code selector,
  command-line runner, and visible replay workflow.
- [Architecture](architecture.md) describes the Rust/C++ boundary, worker
  model, headless execution, and headful promotion.
- [Milestone-backed route search](search.md) documents typed controller
  candidates, population evaluation, lexicographic scoring, and evolution.
- [Native offline reinforcement learning](offline-rl.md) documents the compact
  transition corpus, tree-based fitted Q iteration, trace extraction, and its
  current non-authoritative boundary.
- [Reactive input controllers](reactive-controllers.md) documents the bounded
  timeline DSL, native evaluator, read-only observation boundary, and realized
  tape promotion path.
- [Route timelines, variants, and Git](timelines.md) documents the visual route
  workbench, curated segment variants, exact boundary dependencies, and Git-owned
  lineages.
- [Primitives](primitives.md) defines scenarios, ticks, tapes, controller
  programs, observations, events, oracles, checkpoints, and run artifacts.
- [Determinism and memory fidelity](determinism-and-memory.md) records what
  Aurora's MEM1 model does and does not preserve, plus the sources of
  nondeterminism that need to be controlled.
- [Eye Shredder benchmark](benchmarks/eye-shredder.md) defines the first
  boot-tape and fidelity probe.
- [Intro route benchmarks](benchmarks/intro-route.md) define the first-exit
  frame golf, the path to `demo01_04`, trace scoring, and route-search tools.
- [Roadmap](roadmap.md) breaks implementation into independently verifiable
  milestones.

## Existing foundations

Dusklight already has several useful pieces:

- direct save and stage CLI options in `src/m_Do/m_Do_main.cpp`;
- virtual controller injection through Aurora's `PADSetVirtualStatus`;
- an in-game state-sharing packet containing stage and save data;
- map definitions, an actor spawner, process inspection, collision views, and
  player position/velocity inspection; and
- an original-style JKR heap hierarchy backed by Aurora's MEM1 arena.

The fork now adds an exclusive automation input source, canonical tape
recording/playback, a fixed-step null-render launch mode, and a Rust controller.
Ordinary virtual input still merges by design. Fixed-step runs now control
`OSGetTime`, but alarm dispatch, other asynchronous subsystems, engine-session
worker commands, and portable checkpoints remain unfinished; the current state
packet is a scenario seed rather than a full process snapshot.

## Scope

The first target is repeatable gameplay and UI behavior in this native port.
Exact PowerPC execution and hardware-accurate GX rendering would require an
emulator and are outside the initial platform. When a glitch depends on those
properties, the platform should identify that limitation clearly and still
produce a console-transferable input artifact where possible.
