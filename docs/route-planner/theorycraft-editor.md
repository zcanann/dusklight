# Theorycraft component-transfer and bypass editor

The Route Workbench can author two bounded classes of what-if assumptions from
the **Model context** panel:

- Copy a live component to a new component identity and typed binding.
- Preserve and rebind a live component without changing its payload.
- Assume one catalogued obstruction absent through a typed resolver.

The browser never authors raw catalog records. It sends a typed edit to planner
service v45. Rust derives an exact `ContextScope` from the start state's content
and runtime-configuration digests, emits a `refinement-pack/v15` whose evidence
is explicitly `hypothetical` / `theorycraft`, and composes it as an
`ephemeral_what_if` layer. Component-copy ownership is derived from the source
component instead of being guessed by the client.

Before enabling a component transfer, the workbench previews the source and
destination identities and bindings. The resulting technique or resolver is
then projected into the ordinary planner graph, so reachability and proof
changes use the same engine as every other catalog entry.

## Reversibility and persistence

Web project v3 persists three related values:

1. `theorycraft_base_catalog`, the immutable catalog from before the first edit.
2. `theorycraft_overlays`, the complete reviewable refinement packs.
3. `catalog`, the authoritative composition used by the solver and graph.

Project validation independently recomposes the base and overlays and rejects a
document if the result differs from `catalog`. Each overlay has a Remove control,
and Clear all restores the exact base catalog. Removing an overlay also validates
the active route book, updates an existing refinement-stack digest pin to the new
authoritative stack, and prevents a saved route from retaining a dangling
reference to a removed hypothetical action.

Existing v1 and v2 projects migrate with no theorycraft overlays. An edited v3
project can be saved, reloaded, exported, or reviewed as ordinary canonical JSON;
no assumption is hidden in browser state.

## Engine boundary

`ComposedPlannerCatalog::extend_ephemeral_what_if` accepts only
`ComponentTransform` and `AssumeObstructionAbsent`. It checks pack identity,
dependency digests, conflicts, and monotonic ephemeral precedence before
appending records. Replacement, suppression, binding compilation, and arbitrary
catalog additions remain outside this editor boundary.
