# Determinism and memory fidelity

## Is memory GameCube-backed?

Partially.

Aurora allocates a contiguous MEM1 arena and sets `OSBaseAddress`, `MEM1Start`,
and `MEM1End` around it. `OSPhysicalToCached` and `OSCachedToPhysical` translate
by adding or subtracting that host base. Dusklight then builds its JKR heap tree
inside this arena. This preserves useful properties such as allocation order,
relative offsets, and a single observable game-memory region.

On 64-bit Windows debug builds, Aurora reserves a large aligned region and maps
MEM1 so the low 32 bits of a host pointer look like a cached GameCube address
beginning at `0x80000000`. It also reserves the real low GameCube/Wii ranges as
inaccessible guard memory so accidentally truncated pointers fail loudly.

This is not GameCube hardware or CPU emulation:

- the full pointer is still a 64-bit host address;
- release and non-Windows builds may obtain MEM1 from ordinary `calloc`;
- native class layouts can differ because pointers and ABI rules differ;
- code and static globals use native executable addresses;
- cached and uncached aliases collapse to the same host memory;
- there is no PowerPC MMU, cache behavior, or hardware register map; and
- Aurora translates GX/J2D behavior to a host rendering backend rather than
  reproducing console rendering faults exactly.

The configured Dusklight arena sizes also need not equal retail hardware sizes.
An artifact must therefore use a GC-relative memory token, not a host pointer:

```text
mem1_offset = host_pointer - MEM1Start
gc_cached   = 0x80000000 + mem1_offset
```

The conversion is meaningful only for a validated pointer within MEM1.

Implementation references:

- Aurora's memory contract: [`extern/aurora/include/aurora/aurora.h`](../../extern/aurora/include/aurora/aurora.h)
- MEM1 allocation and guard ranges: [`extern/aurora/lib/dolphin/os/OSMemory.cpp`](../../extern/aurora/lib/dolphin/os/OSMemory.cpp)
- cached/physical translations: [`extern/aurora/lib/dolphin/os/OSAddress.cpp`](../../extern/aurora/lib/dolphin/os/OSAddress.cpp)
- Dusklight's configured arena sizes: [`src/m_Do/m_Do_main.cpp`](../../src/m_Do/m_Do_main.cpp)

## Fidelity classes

Glitches should declare the strongest property they require:

| Class | Property | Native-port expectation |
| --- | --- | --- |
| Gameplay | state machine, collision, timing, and input behavior | primary target |
| Heap-relative | object adjacency and offsets inside a controlled arena | partial; verify per build |
| Absolute-address | exact 32-bit retail address or binary layout | not guaranteed |
| CPU/ABI | PowerPC instruction behavior, 32-bit pointers, or compiler-specific UB | not guaranteed |
| Hardware/render | cache, EFB, GX, or console-only display side effects | not guaranteed |

A test can still be valuable when it stops at an earlier class. For example, a
native run may prove that an out-of-range cursor writes the expected relative
field even if the console-only visual symptom cannot occur in Aurora.

## Fidelity profiles

The fork should expose profiles rather than silently mixing safety changes with
original behavior:

- `safe`: normal port behavior and bounds/UB fixes;
- `fidelity`: intentionally reproduce selected original behaviors inside
  constrained memory regions;
- `instrumented`: add guards, write tracing, assertions, and sanitizers to
  explain a result.

Selected original bugs should not be restored by relying on host C++ undefined
behavior. Express the operation explicitly against a byte layout or MEM1 offset,
validate the allowable corruption region, and trap writes that would escape
game-owned memory. Each compatibility shim needs a source citation, a focused
test, and a capability flag in artifacts.

`TARGET_PC`, `AVOID_UB`, safe string helpers, and similar conditionals are part
of the fidelity audit. Their presence does not mean they should all be removed;
each one needs a deliberate, tested policy.

## Determinism threats

### Input

Virtual input currently merges with physical input. Automation must select an
exclusive source and record the canonical state after host mapping and before
the game consumes it.

### Time and pacing

`OSGetTime` currently derives from a host steady clock, while the main loop also
waits for video retrace. Introduce a logical clock for game-visible time and
separate it from profiling time. An unpaced run advances logical time by the
same amount per `SimTick` as a realtime run.

The current host-clock implementation is in
[`extern/aurora/lib/dolphin/os/OSTime.cpp`](../../extern/aurora/lib/dolphin/os/OSTime.cpp).

### Random number generation

Record and control every game RNG stream, including initial state and call
count. State hashes should make an unexpected extra call visible. Do not seed
gameplay from wall time in deterministic modes.

### Threads and asynchronous subsystems

DVD/loading, audio, movie, and host threads can change ordering. Deterministic
mode needs explicit completion points or a deterministic job schedule. A
checkpoint cannot be considered valid while it contains unknown in-flight host
work.

### Floating point and compiler configuration

Record architecture, compiler, optimization, floating-point flags, and feature
flags. Cross-build replay may be informative, but same-build replay is the
initial guarantee. Canonical hashes should normalize values only when the
normalization is part of the declared test semantics.

### Rendering and draw traversal

Headless execution must initially preserve game-visible work performed during
draw traversal while replacing presentation with a sink. A parity harness runs
the same tape headful and headless and compares periodic hashes and events.
Only work proven irrelevant may be disabled.

## Canonical state hashes

Hash selected semantic state, not raw process memory. Raw memory contains host
pointers, padding, allocator bookkeeping, and nondeterministic subsystem data.
The initial hash should include:

- player transform, motion, action, animation, room, and form;
- stage transition and important save/event flags;
- controlled RNG states and call counts;
- stable actor identities and selected actor state;
- relevant UI state for menu tests; and
- explicitly watched GC-relative memory ranges.

Use a full diagnostic trace around the first mismatching hash to locate the
cause. The hash is a divergence alarm, not a replacement for observations.

The first implemented profile is `core-typed-facts/v1`. `huntctl` derives one
hash for every retained gameplay-trace boundary and seals the series to the
source trace, trace version, boot origin, tick rate, field profile, observation
phase, simulation tick, and tape frame:

```sh
cargo run --manifest-path tools/huntctl/Cargo.toml -- \
  trace state-hashes build/run/gameplay.trace \
  --output build/run/state-hashes.json

cargo run --manifest-path tools/huntctl/Cargo.toml -- \
  trace compare-state build/reference/gameplay.trace build/trial/gameplay.trace
```

Comparison requires compatible profiles, boot origins, and tick rates. It
reports the first changed or missing boundary and both available hashes, which
gives parity and reset experiments an exact point at which to request detailed
trace evidence.

The v1 profile includes the status and value of the current typed-fact query
aperture: stage name, room, and spawn; player existence, Link identity, and
position; event-running state and event ID; and player do-status, talk partner,
and grabbed actor. Unavailable, absent, truncated, and invalid statuses are
hashed rather than silently treated as values.

This is not a whole-game-state hash. It does not yet cover player motion and
animation detail, save/event flags beyond the active event ID, RNG streams,
the actor catalog, UI state, collision/contact state, or watched memory ranges.
Equal v1 hashes establish equality only for the documented typed-fact aperture;
a differing hash proves that aperture diverged and should trigger trace diff.
