# Offline world spatial queries

`huntctl world spatial-index` builds a deterministic per-room median-AABB BVH
from the immutable reconstructed triangles in a world inventory. The artifact
is bound to the exact inventory SHA-256 and records every excluded degenerate
surface. Like the inventory reader, this code is Rust-only and offline: it does
not launch Dusklight, link gameplay code, call native collision helpers, or
read/write a live process.

## Coordinate contract

Every query requires `--room`. Coordinates use
`room-kcl-authored/v1`; room transforms have not been decoded, so combining
room coordinate spaces would be an unsupported guess. Cross-room queries are
therefore rejected structurally rather than producing plausible-looking wrong
answers.

The index uses source `f32` triangle vertices and deterministic `f64` query
math. Per-room primitive identity tables are stable-ID sorted. BVH splits use
the largest centroid extent with X/Y/Z tie order, median partitioning, stable
identity tie-breaking, leaf capacity eight, and canonical preorder nodes.

```powershell
cargo run --quiet --manifest-path tools/huntctl/Cargo.toml -- world spatial-index `
  --stage-dir orig/GZ2E01/files/res/Stage/F_SP103 `
  --stage F_SP103 `
  --output build/world/F_SP103.spatial.json
```

The checked F_SP103 index has digest
`2ad975eee45193b4325bb420a7ba5a78d533bed80cbcfeace29dcc5418e73834`:
8,566 indexed room-0 surfaces, 2,224 indexed room-1 surfaces, and four
explicit exclusions. Of the 40 inventory load-trigger joins, 39 have spatial
geometry; the remaining join references retained degenerate room-1 prism 2225.

## Query operations

Nearest point-to-triangle query:

```powershell
cargo run --quiet --manifest-path tools/huntctl/Cargo.toml -- world query point `
  --stage-dir orig/GZ2E01/files/res/Stage/F_SP103 --stage F_SP103 --room 1 `
  --point "-2037.3324,729.72,-4264.551" --max-distance 0.001 --limit 4 `
  --destination-stage F_SP104
```

Without `--max-distance`, point queries use best-first BVH traversal and stop
only after the remaining lower bound is strictly worse than the current Kth
hit. Equal bounds are still visited so stable-ID tie-breaking is independent
of tree topology. With a maximum distance, the report counts every exact hit
inside the radius. Results include the closest point, precise Euclidean and
signed-plane distances, full authored collision facts, and any trigger join.

AABB neighborhood broad phase:

```powershell
huntctl world query aabb ... --room 1 --min X,Y,Z --max X,Y,Z --limit 32
```

This returns triangles whose AABBs overlap the query box. The report labels
the operation `triangle-aabb-overlap-broad-phase`; it does not claim exact
triangle/box intersection or region containment.

Finite double-sided ray query:

```powershell
huntctl world query ray ... --room 1 --origin X,Y,Z `
  --direction X,Y,Z --max-distance 1000 --limit 16
```

Direction is normalized explicitly. Hits contain precise distance, position,
barycentric coordinates, and `front_facing`. Double-sided intersection is an
offline inspection primitive, not a claim about the game's one-sided material
or movement collision behavior.

All operations enforce a result limit of 1 through 256, report node visits and
(where applicable) triangle tests, sort by the unrounded `f64` metric then
stable ID, and expose result-capacity truncation. Reports separately count
degenerate records that would have matched the filter, so absent geometry is
never mistaken for an empty authored result. Filters are applied before ranking
and can select:

- all spatial load triggers;
- one exact trigger stable ID; or
- an exact destination stage, room, and/or point.

This matters for route objectives. At a checked point 100 units above Ordon's
exit triangle, the globally nearest polygon is prism 2187, while the nearest
`F_SP104` trigger is prism 2217. A global-nearest-then-filter implementation
would return the wrong objective surface; the acceptance test locks in the
correct pre-ranking filter behavior.

## Limits and next use

The index contains collision surfaces, not volumes, regions, navigation, or
gameplay contacts. It cannot yet answer water/trigger containment, capsule
sweeps, clearance, ledges, seams, room connectivity, or path queries. Those
need decoded authored shapes or explicitly versioned inferred topology.

The next controller-facing step is a small content-addressed world-view
artifact that selects a fixed set of nearby/objective surfaces and missingness
masks. Full inventory or BVH data must not be copied into every trace frame or
fed wholesale to a model.
