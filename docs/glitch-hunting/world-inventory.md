# Offline world inventory

`huntctl world inventory` builds a deterministic, content-addressed view of
authored stage data without launching Dusklight or reading a live process. The
reader consumes immutable RARC, DZS/DZR, KCL, and PLC bytes. It does not link
against gameplay code, issue collision queries, populate game caches, or write
game state.

## Build an inventory

From the repository root:

```powershell
cargo run --quiet --manifest-path tools/huntctl/Cargo.toml -- world inventory `
  --stage-dir orig/GZ2E01/files/res/Stage/F_SP103 `
  --stage F_SP103 `
  --output build/world/F_SP103.inventory.json
```

The output is canonical compact JSON using schema
`dusklight-world-inventory/v2`. Host paths and timestamps are excluded, so the
same source bytes produce the same artifact digest. The checked GZ2E01 content
currently produces SHA-256
`e4f63e67da30035c8d15cc92595bc384dcc84adbe7a5d361acbcb0302bdcd743`.

Version 1 artifacts remain readable and retain their original canonical bytes.
Version 2 adds checked authored room paths without changing any v1 meaning.

## What version 2 contains

- every parsed chunk directory entry from `stage.dzs` and each `room.dzr`;
- recognized actor, treasure, scaled/door, and player-spawn placements with
  source transform, parameters, set ID, layer, and raw record bytes;
- every SCLS exit with its normalized destination and retained raw fields;
- every fixed-layout `RPAT` path and paired `RPPN` point, including exact
  arguments, optional next-path links, closed/switch fields, checked point
  spans, authored coordinates, and raw record bytes;
- every addressable KCL prism with authored height, source indices, material
  attribute, decoded PLC words, and a content-stable ID;
- reconstructed triangle and plane geometry where the retail data permits it;
  otherwise an explicit `degenerate` status and reason; and
- inferred collision-load-trigger records joining a PLC exit ID to the
  same-room SCLS record bearing that index.

Stable placement and exit IDs combine the DZS/DZR content digest, chunk tag,
and record index. Collision IDs combine the KCL and PLC digests with the prism
index. Composite trigger IDs hash the two source IDs. They deliberately do not
use runtime addresses or process IDs.

The load-trigger join is marked `inferred_semantics: true`: it is a structural
interpretation of authored records, not proof that a particular collision
surface will be consulted on a given frame. Runtime proof remains the job of
the read-only cached-collision trace channel.

## F_SP103 content acceptance

The optional content-golden test runs automatically when the original disc
tree is present and otherwise reports a skip:

```powershell
cargo test --manifest-path tools/huntctl/Cargo.toml `
  world_inventory::tests::real_f_sp103_inventory_matches_content_golden_when_disc_is_present
```

The checked fixture contains three archives, 1,442 placements, 48 player
spawns, 44 exits, 82 paths, 426 path points, 10,794 addressable collision
prisms, and 40 inferred load triggers. Four authored prisms cannot be
reconstructed; all four remain in the artifact with their source facts and
explicit failure reasons.

The acceptance test also proves the route-critical join:

```text
room 1 KCL prism 2217
  -> PLC attribute 19, raw exit ID 1
  -> room 1 SCLS record 1
  -> F_SP104, room 1, point 0, layer -1, wipe 19
```

This gives controllers and inspectors an offline geometry/source reference.
Compiling nearby surfaces or signed-distance features into a bounded per-task
observation is separate work; the full 11 MB world artifact should never be
copied into every frame or fed wholesale to a model.

## Current limits

A separate content-addressed per-room spatial index provides nearest-surface,
AABB broad-phase, and finite-ray inspection over the decoded triangles.
Authored paths are immutable proposals, not a claim that Link can traverse
them in the current state; native execution remains authoritative. Water and
trigger volumes, cameras, event tables, switch semantics beyond the retained
path field, region containment, and broader stage formats still need explicit
decoders. Unknown DZS/DZR chunks remain listed rather than guessed at or
silently presented as decoded.
