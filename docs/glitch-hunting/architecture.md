# Architecture

## Process model

The system has a Rust control plane and one native Dusklight process per
simulated client.

```text
huntctl (Rust)
  |-- corpus, scheduling, minimization, watchdogs, artifacts
  |-- worker protocol (coarse batch commands)
  `-- N persistent dusklight-worker processes (C++)
        |-- deterministic tick driver
        |-- exclusive controller input
        |-- observations, events, state hashes, oracles
        `-- scenario reset, replay, and checkpoint support
```

Separate processes provide crash containment and allow the operating system to
place workers on different cores. They also avoid exposing a large unstable C++
ABI to Rust. A worker stays alive across many candidates and executes batches
in-process, so process startup and IPC are not paid per run or per frame.

The initial protocol can use a versioned binary control stream over local pipes
or sockets. Add memory-mapped command/result rings only after measurements show
that result transfer is material. Never send every controller frame through
IPC: upload a tape or controller program once, then ask the worker to run a
range of ticks.

## Responsibilities

### Rust control plane

- discovers workers and validates their capabilities and build IDs;
- schedules independent candidates and multi-client groups;
- stores the seed corpus and run artifacts;
- performs mutation, crossover, minimization, novelty bookkeeping, and search;
- enforces timeouts, restarts crashed workers, and records crash metadata;
- chooses CPU affinity and concurrency based on measured throughput; and
- launches a headful replay from an interesting artifact.

Rust should not call into C++ once per tick. Search policies generate or mutate
compact controller programs and tapes, then submit batches.

### C++ worker

- owns the game process and calls the game loop directly;
- applies controller state before the game consumes input;
- advances exactly one logical simulation tick at a time;
- virtualizes pacing and time sources used by game logic;
- captures game-specific observations without serializing the entire world;
- evaluates cheap per-tick oracles close to the data;
- produces stable state hashes and event streams; and
- resets or restores the scenario without leaking state between candidates.

Game-specific access belongs here because the relevant types, actors, heaps,
and symbols are C++ implementation details. Expose stable values and IDs in the
protocol, never live pointers.

## Worker protocol

Every message begins with a protocol version, request ID, build identity, and
payload length. The minimum command set is:

- `Hello`: negotiate versions and report capabilities such as headless,
  checkpoint tier, fidelity profile, controller count, and render capture;
- `LoadScenario`: establish save, stage, room, spawn, settings, RNG, and other
  preconditions;
- `UploadTape` / `UploadProgram`: install an immutable input source;
- `Run`: execute a tick range with requested observations and oracles;
- `Reset`: return to the loaded scenario and verify its initial hash;
- `CreateCheckpoint`: create a named acceleration point when supported;
- `Replay`: run an artifact and optionally begin presentation at a given tick;
  and
- `Shutdown`: terminate cleanly.

Results include the terminal reason, tick count, event stream, requested
observations, state hashes, oracle scores, and any crash or fidelity flags.
Large traces and images are content-addressed files referenced by the result,
not embedded in routine protocol messages.

## Fixed-tick execution

One `SimTick` is one game update at the configured logical video mode. Fast
execution removes retrace waits, audio-device waits, and presentation, but does
not increase the logical frame rate or multiply movement by elapsed wall time.

The first implementation should make the existing main loop callable in three
modes:

- `realtime-headful`: normal presentation and pacing;
- `unpaced-headful`: presentation without retrace pacing, useful for replay and
  debugging; and
- `unpaced-headless`: no window or presentation, with the same game traversal
  and logical clock.

The draw traversal cannot initially be deleted wholesale. Some game draw paths
also prepare matrices or other state used later. Start with a render sink that
accepts the traversal but discards backend output, then remove work only behind
headful/headless parity tests.

## Input ownership

Automation needs an exclusive input mux. Aurora currently merges virtual and
physical controller state, which makes a tape sensitive to whoever touches a
real controller. Each port should select exactly one source per tick:

- physical;
- tape;
- controller program;
- remote multi-client input; or
- neutral/disconnected.

Recording happens after source normalization and before the game reads the
pad. Replaying a tape must bypass dead-zone recalculation and host controller
mapping so the game receives the exact recorded signed stick and trigger bytes.

## Multi-client runs

A client is a worker process with its own memory, scenario, input, and event
stream. The Rust scheduler groups workers under a deterministic barrier when a
test needs coordinated clients. The barrier advances logical ticks; it does not
try to synchronize wall-clock rendering.

Cross-client messages must be captured as ordered events with sender, receiver,
logical tick, and payload digest. Network delay and loss are explicit schedule
inputs so a multi-client run can be replayed.

## Promoting a run to headful

A headless run never attempts to transplant native pointers into a visible
process. Promotion is replay-based:

1. launch a headful worker with the artifact's exact build and scenario;
2. replay the tape unpaced and without presentation up to a chosen tick;
3. verify checkpoint hashes while replaying;
4. enable presentation, audio if desired, and interactive inspection; and
5. retain automation ownership unless the user explicitly takes over a port.

Later checkpoint tiers may shorten the prefix replay, but the tape remains the
portable source of truth.

## Throughput strategy

Optimize in this order:

1. remove pacing and presentation costs;
2. keep workers warm and reset cheaply;
3. batch candidates and evaluate cheap oracles in C++;
4. reduce observation volume;
5. add process-level parallelism and CPU affinity;
6. add safe checkpoints or a forkserver where the platform permits it; and
7. profile before introducing shared-memory transport or custom allocators.

The useful metric is deterministic candidate-ticks per second, not rendered
frames per second.
