# Project scope and benchmark selection

Dusklight's near-term success criterion is a trustworthy reproduction toolkit
for a small, human-selected set of Twilight Princess glitches. It is not
whole-Skybook replication, exhaustive reverse engineering, autonomous discovery
at scale, or a distributed RL platform.

## Core scope

- Preserve exact controller input and cold-replayable proof.
- Support clean boot and explicit stage-boot fixtures.
- Expose only observations required by an approved benchmark.
- Define a semantic success oracle and retain failure evidence.
- Reproduce 3–5 selected pages, starting with the easiest useful one.
- Pause after each reproduction for a human scope decision.

The checked Skybook manifest is reference material. Importing a page does not
make it a task. Unselected pages receive no generated requirements, readiness
state, implementation work, or implied promise of reproduction.

## Selection gate

A human selects each pilot page. Prefer a short documented setup, stage-boot
feasibility, controller-only execution, a simple semantic success condition,
native-port fidelity, and reuse of existing observations/actions.

Default complex memory corruption, multi-map setup chains, console-only
rendering, poorly sourced pages, and hardware-specific timing to `deferred` or
`won't-do`. Override that default only through an explicit human decision.

## Reach goals

Learned proposal systems, autonomous novelty campaigns, intervention research,
portable checkpoints, whole-subsystem observation, generalized graphical query
tools, and emulator/console transfer suites are optional. Activate one only
when a selected benchmark demonstrates a concrete need and simpler methods are
measured inadequate.

## Won't-do in the current scope

Do not build distributed workers, deterministic multi-client networking, NUMA
schedulers, OS process snapshots/forkservers, or a requirements/mechanism graph
for the entire Skybook corpus. These may remain design notes, but they are not
unchecked obligations and must not drive implementation.
