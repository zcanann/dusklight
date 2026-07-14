# Automation primitives

These primitives form the stable boundary between search code and the game.
Their serialized forms must be versioned and independent of C++ padding,
pointer size, and host endianness.

## Build identity

Every artifact records enough identity to reject a misleading replay:

- Dusklight commit and dirty-tree digest;
- Aurora commit;
- compiler, target, build type, and feature flags;
- game data/region digest;
- protocol and artifact schema versions; and
- fidelity profile.

## Scenario

A `Scenario` describes repeatable initial conditions, not just a map:

- save image or named fixture;
- stage, room, layer, entrance/start point, and player form;
- inventory, flags, health, magic, rupees, and other relevant save state;
- initial RNG streams;
- logical video mode and language;
- game settings that can affect input or timing;
- optional actor or memory fixtures; and
- required capabilities and fidelity profile.

Scenario setup may observe a declared ready condition before the replay clock
starts, then emits an initial state hash. Once tape playback starts, readiness
observations are assertions only: they may fail a run but must never delay,
skip, repeat, or choose input.

## SimTick

`SimTick` is a monotonically increasing logical update index. All inputs,
events, observations, network schedules, and checkpoint hashes are keyed to it.
Wall-clock timestamps may be recorded for profiling but never define replay
behavior.

## InputFrame and InputTape

An `InputFrame` holds the exact game-visible state of every emulated controller
port for one tick:

- digital button bits;
- signed 8-bit main-stick X/Y;
- signed 8-bit sub-stick X/Y;
- unsigned 8-bit analog triggers;
- unsigned 8-bit analog A/B;
- connection and error state; and
- explicitly versioned port-specific extensions, if any.

Do not serialize `sizeof(PADStatus)`: its padding and PC-only fields are build
details. Use a canonical packed schema.

An `InputTape` contains build-independent metadata, a scenario reference,
run-length encoded frames, named markers, and optional expected hash/event
checkpoints. It supports lossless splice, trim, and neutral-frame insertion.
The tape is the final proof produced by any higher-level controller. Its input
at tick N is fixed before replay begins; it contains no observation-dependent
waits or branches.

## ControllerProgram

A `ControllerProgram` drives search and compactly generates candidate tapes.
It is not itself a TAS proof. Initial operations may include:

- hold/release buttons for a tick range;
- set exact stick or trigger bytes;
- wait for a bounded semantic condition;
- choose a value or timing from a declared search range;
- repeat and branch on an observation; and
- invoke a parameterized motion segment.

Motion segments may later include stick-space splines, angle/speed targets, roll
spacing, and state-feedback controllers. A successful run must materialize the
exact raw `InputFrame` sequence it actually used. Verification replays that
flattened sequence with all program waits and branches removed. A spline is a
search convenience, not a replay format.

Programs must be bounded: every wait, loop, and branch has a maximum tick count
and deterministic timeout result.

## Observation

An observation is a typed, stable value sampled on request. The first useful
set includes:

- player position, velocity, facing, animation, action/state, form, and room;
- camera and lock-on state;
- stage transition and loading state;
- controller state after normalization;
- RNG state or call counters;
- selected actor IDs, types, transforms, health, and process state;
- collision contacts and relevant polygon metadata;
- UI screen, selection, cursor, and text-entry state;
- heap usage and GC-relative offsets for watched objects; and
- canonical state hashes.

Stable actor handles combine a spawn/event identity and generation. Native
pointers are diagnostic values only and never enter portable artifacts or
hashes.

Placed actors use a portable composite identity: stage, home room, actor type,
and map-authored set ID. A runtime process ID is exact only for that actor's
lifetime in one engine session. Actor catalogs may add parameters, parent ID,
and home transform for diagnosis and disambiguation; they never use a pointer
as identity.

## InterventionTape

An `InterventionTape` is a separately gated sequence of typed game-state writes
used to test causal hypotheses, such as whether an enemy collision can push
Link across a fence. It is never folded into an `InputTape` and never presented
as an unmodified replay.

Each intervention declares its exact simulation phase, stable actor selector,
field-level operation, precondition, and bounded duration. Execution records
selector resolution plus before/written/after values. Baseline and treatment
runs retain identical scenarios and controller inputs so an oracle can attribute
the changed outcome to the declared intervention.

Intervention-built discoveries become normal glitch proofs only after the
required setup is reproduced without mutation. Until then they are useful
existence or mechanism evidence, not evidence that the setup is naturally
reachable.

## Event

Events make sparse behavior cheap to search and easy to inspect. Examples are:

- action or animation changed;
- room/stage transition requested or completed;
- actor spawned, deleted, damaged, or changed process state;
- collision contact began or ended;
- position crossed a boundary or moved discontinuously;
- input was consumed by a different UI/game state than expected;
- watched memory was written outside its intended field;
- crash, assertion, timeout, NaN, or invalid handle; and
- headful/headless state hashes diverged.

Each event records a tick, stable subject IDs, a schema version, and a compact
payload.

## Oracle

An oracle turns a run into a terminal classification and optional score. Useful
built-ins include:

- reached a target region, room, action, animation, or flag;
- crossed collision without the expected transition;
- exceeded plausible displacement, speed, or vertical range;
- entered out-of-bounds or invalid state;
- corrupted a watched field or heap guard;
- crashed or hung;
- produced a novel event/state signature; and
- diverged from a control replay or from headful execution.

Keep common oracles in C++ so every tick does not cross the process boundary.
Rust composes their results and runs expensive corpus-wide novelty decisions.

## Checkpoint

Checkpoint support is tiered because a native process is not trivially
serializable:

1. `Scenario`: reload save/stage and replay a prefix; portable and mandatory.
2. `GameState`: serialize explicitly supported game state with pointer fixups;
   faster but incomplete until proven otherwise.
3. `ProcessSnapshot`: restore memory, thread, and subsystem state; fastest in
   theory and highly platform-specific.

Every checkpoint has a build ID, parent scenario, tick, canonical state hash,
capability requirements, and a validation replay. A checkpoint that cannot
reproduce its next validation window is rejected.

## RunArtifact

A `RunArtifact` is the unit stored in the corpus. It contains:

- build identity and scenario;
- exact input tape and any source controller program;
- seed/mutation lineage;
- terminal reason, oracle results, and novelty signature;
- selected event and observation traces;
- periodic canonical state hashes;
- crash report or memory-write trace when applicable;
- optional screenshots/video from a verified headful replay; and
- performance counters that are clearly separated from logical time.

Artifacts should be content-addressed, immutable, and minimizable. A minimized
artifact retains the same semantic oracle and required fidelity capability.
