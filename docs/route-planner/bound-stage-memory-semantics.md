# Bound stage-memory semantics

The planner treats the live `dSv_memBit_c` payload as one raw component owned by
the active runtime file's current stage bank. Native projection emits an exact
32-byte observation supplied through `loaded_stage_memory_bytes` as
`flags.loaded-stage-memory` with:

- component kind `dungeon_memory`;
- binding `stage { stage }`;
- lifetime `stage_load`; and
- owner `stage_bank { runtime_file_id, stage }`.

The separately observed `dSv_danBit_c` and current-room switch label arrays are
diagnostic views, not raw backing stores. They use custom component kinds and
cannot satisfy a bound stage-memory read or write. A producer that has not
captured the exact 32-byte payload leaves the live backing absent, causing these
operations to evaluate or execute as unknown/failing rather than mutating a
label-indexed array.

It does not copy dungeon-local values into the runtime-file inventory component.
That would create two independently mutable truths and could let a Forest Temple
key survive under the wrong backing-store semantics.

## Source-audited layout

For the audited GZ2E01 source, `dSv_memBit_c` is 0x20 bytes. The relevant tail is
defined in `include/d/d_save.h` and manipulated in `src/d/d_save.cpp`:

| Byte | Meaning | Mask/value |
| --- | --- | --- |
| `0x1c` | Fungible small-key count | full byte |
| `0x1d` bit 0 | Dungeon map | `0x01` |
| `0x1d` bit 1 | Compass | `0x02` |
| `0x1d` bit 2 | Boss key | `0x04` |
| `0x1d` bit 3 | Stage boss defeated | `0x08` |
| `0x1d` bit 4 | Stage heart container | `0x10` |
| `0x1d` bit 5 | Boss demo seen | `0x20` |
| `0x1d` bit 6 | Ooccoo/warp note | `0x40` |
| `0x1d` bit 7 | Miniboss defeated | `0x80` |

Other builds must either share an evidenced layout or provide their own scoped
rules. A friendly label never makes this table universal by itself.

## Reads

`bound_raw_bits` selects by component kind and a semantic binding reference
before reading a byte range. A reference may name an exact binding or resolve at
evaluation time to the active runtime file, current stage, or current room. The
resolved value is still a concrete `ComponentBinding`; components never acquire
a dynamic or implicit owner.

A small-key guard can therefore compare byte `0x1c`, width 1, mask `0xff`
against zero without naming either the transient live component ID or a stage
copied into every imported rule. A boss-key guard reads byte `0x1d`, width 1,
mask `0x04`. After a scene or runtime-file change, the same rule resolves against
the new environment and cannot accidentally keep reading the authoring context.

Resolution fails to unknown when:

- no live component has the requested kind and binding;
- more than one component claims that kind and binding;
- the selected payload is not raw;
- the byte range is out of bounds; or
- any selected bit is unknown.

Rebinding the payload changes which bound read can see it without changing its
bytes. This is the intended behavior for ordinary stage-bank loads and for an
explicit hypothetical wrong-bank transfer.

## Count mutations

`adjust_bound_raw_unsigned` resolves the same binding-reference forms and applies
a signed delta to the uniquely selected raw component. It requires a 1–8 byte
fully known little-endian unsigned field and rejects missing, ambiguous, unknown,
out-of-range, underflowing, or overflowing updates. The containing transition
batch remains atomic.

Consequently, any same-bank key pickup may increment `0x1c`, and a keyed door may
decrement it. Provenance records the concrete producer or consumer action. A
route that goes around the door performs neither operation, while a bank rebind
automatically changes which transitions can read or mutate the count.

Persistent unlock switches, pickup/chest bits, pending HUD key deltas, and live
door collision remain separate state. This mechanism does not invent one
universal door program; actor-family guard and reconstruction audits are still
required.

## Masked writes and unknownness

`write_bound_raw` and `invalidate_bound_raw` use the same binding-reference and
exact-one selection rules. A writer changes only the selected bits and marks
those bits known; invalidation preserves the bytes while clearing only the
selected knownness bits. This lets imported message events, item grants, and
door switches address their real backing without a fixture-specific component
ID. Missing, ambiguous, non-raw, and out-of-range targets fail the entire
operation batch rather than silently writing a different store.
