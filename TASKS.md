# Learning workbench: recording and playback

This milestone is complete. There are no unfinished learning-product tasks in
this file.

## Product contract

An LLM or another programmatic client:

1. selects an authenticated starting checkpoint and authored goal;
2. launches learning with the runtime's finite catalog of applicable actions;
3. receives the recorded checkpoint/state/action graph produced by execution;
4. inspects recorded decisions and evidence; and
5. replays a recorded edge or root-to-edge path as exact controller input.

The browser workbench is a recorder, inspector, and playback surface. It does
not author tactics, controller input, action graphs, or blueprint nodes.
Tactics and compositions are runtime actions supplied through code or the
programmatic contract. Interactive exploratory graph authorship belongs to the
route planner in `TASKS_ROUTE_PLANNER.md`.

## Acceptance status

| Requirement | Status |
|---|---|
| Launch learning from the selected authenticated start and goal | Complete |
| Learn by choosing applicable runtime actions and branching from retained checkpoints | Complete |
| Record checkpoint states, action edges, outcomes, rewards, and exact input ranges | Complete |
| Project the recorded graph without loading full evidence eagerly | Complete |
| Load authenticated decision details on demand | Complete |
| Replay an exact recorded edge or root-to-edge candidate path from ordinary process boot | Complete |
| Pause, resume, cancel, and clean up a campaign without losing retained evidence | Complete |
| Export a successful exact tape and prove it from cold boot | Complete |
| Keep manual TAS, tactic, and graph authorship out of the workbench | Complete |

The first no-demonstration proof reached the terminal, froze a greedy policy,
exported its exact tape, and reproduced the same semantic gameplay state across
two cold runs:

`docs/glitch-hunting/benchmarks/ordon-tactic-q-first-proof-20260724.json`

The workbench implementation and contract tests live in:

- `tools/huntctl/crates/workbench/src/tactic_route_runtime.rs`
- `tools/huntctl/crates/workbench/src/server.rs`
- `tools/huntctl/assets/route_workbench.html`
- `tools/huntctl/crates/workbench/src/tests.rs`

## Not tracked as product requirements

Demonstration-seeded comparisons, throughput tuning, multi-seed research
studies, and route-time refinement are optional experiments. They should become
tasks only when a concrete product question or measured bottleneck requires
them.
