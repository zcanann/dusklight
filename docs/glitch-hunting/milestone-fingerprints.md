# Milestone boundary fingerprints

Every hit in a native `dusklight.automation.milestones` result includes explicit boundary evidence
and a `dusklight.milestone-boundary/v2` fingerprint. Search tooling can use this to distinguish a
strict frame improvement from a faster arrival that leaves a materially different known state.

The digest is XXH3-128 over a fixed v2 byte layout. Integers and raw IEEE-754 binary32 float bits
are little-endian; stage names occupy eight zero-padded bytes. Tick number, tape frame, process
addresses, padding, and host time are deliberately absent. Thus the same captured evidence hashes
the same way across processes and machines.

V2 adds explicit event-name-hash presence. The current const-only observer reports it unavailable
and serializes `name_fnv1a: null`; it does not call the event manager's non-const
`getRunEventName()`. V1 artifacts remain readable, but v1 and v2 fingerprints are intentionally
different identities and cannot anchor each other's descendants.

## Included state

The JSON preserves the values used by the digest so a mismatch is explainable rather than merely
an opaque hash difference:

- live stage name, room, resolved layer, and start point;
- player presence and Link identity, actor/process/procedure IDs, position, velocity, forward
  speed, current angle, and shape angle;
- event running flag, event ID, mode, status, map-tool ID, and explicit event-name-hash
  presence/value;
- enabled next-stage flag and its stage, room, layer, and point;
- both native `cM` random streams: snapshot/algorithm version, stream ID, all three generator
  states, and call count.

The RNG values come from the backing state used by `cM_rnd`/`cM_rnd2` and their `F`/`FX`
helpers (`g_primary` and `g_secondary` in `c_math_rng.cpp`) through
`capture_game_rng_snapshot()`. No host timestamp or replacement pseudo-random source is involved.
Capturing them does not advance either stream.

## Not included

Version 2 does not claim a complete emulator or save-state hash. It currently omits:

- non-player actor population and individual actor state;
- collision, background, physics-contact, and trigger internals;
- camera, renderer, audio, particle, and UI state;
- save data and the complete event, room-switch, item, and temporary-flag arrays;
- asynchronous DVD/resource-loader queues and host scheduling state;
- heap layout, pointers, and process addresses.

An equal fingerprint therefore means equality for the documented boundary fields, not proof that
all future execution is identical. A differing fingerprint proves the captured boundaries differ.
In particular, two arrivals with different RNG snapshots are separate search lineages: the faster
one must not automatically dominate the slower one merely by frame count. Archive or compare them
as distinct boundary cells, then evaluate descendants from each.

The top-level milestone result is schema version 2. Current consumers accept immutable v1 results
for inspection, but new runs emit v2 and must require the matching nested schema string and
canonical-encoding label.
