# Twilight Princess Route Planner Runtime

This is the independent application boundary for the causal route planner. It
does not register commands with Huntctl and does not use the TAS timeline
workbench's graph or playback schemas.

The Rust planner engine lives in `crates/engine`. This tool owns its schemas,
CLI, reports, input contracts, and future server/editor protocol. It has no Rust
dependency on Huntctl/TAS crates. Existing producers can exchange compatible,
content-addressed data at the wire boundary; any future TAS consumption of the
planner must be initiated downstream by that project.

```text
cargo run --manifest-path tools/route-planner/Cargo.toml -- help
```

The planner CLI currently owns thirty-eight operations:

- `compose` validates deterministic layered refinement stacks and emits a
  canonical composed fact/mechanics catalog. `--pack`, `--route-overlay`, and
  `--what-if-overlay` are distinct precedence domains; disposable overlays
  cannot silently become installed knowledge. Authored obstruction selectors
  bind to concrete actions during composition, producing solver/graph
  dependencies without route-book wiring.
- `compile-cutscene` validates a phase-level cutscene program and compiles each
  normal, skip, interruption, scene-change, or resource-failure branch into an
  ordinary causal transition. Confirmed prefixes remain ordered and unaudited
  suffix targets become explicit unknownness.
- `compile-return-place-mechanics` emits the exact GZ2E01/English tower
  `Savmem` writer, its raw event-bit activation and `NO_TELOP` gate, and the
  ordinary dynamic savewarp reader as a standalone mechanics catalog. Actor
  execution remains a live-state dependency rather than an assumed success.
- `compile-title-boundary-mechanics` emits the exact successful GZ2E01 reset
  prefix, route-relevant title-file-0 opening projection, source-audited title
  input/request steps, and normal file-select create projection. Pending F_SP102
  remains non-traversable and cannot reach phase 4 without an explicit process-
  scheduler observation. Likewise, the title actor's name-scene request does not
  become an active process until independently observed. The two save-domain
  initializers replace only audited backing payloads; the first can enter a
  fresh title-origin lifetime, while neither can mutate unrelated inactive
  stores or physical-slot images.
- `construct-message-flows` selects every message group for one exact
  content/runtime/language profile and emits canonical source programs without
  guessing unaudited backing stores.
- `compile-message-flows` turns those exact resources and optional audited
  overlays into ordinary fact/mechanics catalogs and seals the result with a
  fact-pack manifest.
- `compile-message-entries` joins authored callers to an exact stage resource,
  actor placement, message resource, and flow label. Its entry transitions keep
  interaction feasibility separate and can project the audited speaker context
  into subsequent backing-store references.
- `edit-route-book` applies an atomic, expected-digest-checked batch of typed
  route-book edits and emits a fully revalidated canonical revision.
- `diff-state` compares two executable states across a named boundary, retaining
  raw/component deltas and recomputed friendly-fact deltas. Binding-only changes
  are identified separately from payload changes, so identical bytes receiving
  a new semantic interpretation remain visible.
- `diff-orig` compares two canonical extracted bundles at both archive-byte and
  decoded-record levels while sealing both exact input bundle digests. Optional
  left/right locale selection pairs message groups and ignored candidates across
  language bundles; missing groups, zero-group locale coverage, and ignored
  message archives remain explicit instead of implying equality.
- `extract-world` converts generic canonical world artifacts into conservative
  planner facts and unresolved physical obligations.
- `extract-resource` performs bounded Yaz0/RARC extraction of one uniquely
  named resource from user-supplied retail data.
- `scan-orig` discovers an extracted GameCube/Wii disc from either its game
  root or a parent `orig/` directory, reads the product/revision header, hashes
  every regular file below the extracted `sys/` and `files/` trees, and emits a
  path-normalized sealed scan. It never trusts the directory name and rejects
  ambiguous roots and symlinks.
- `extract-orig` verifies that scan against the bundled exact-build registry by
  default, then decodes every recognized stage/room archive and message bundle
  into one canonical derived artifact plus a fact-pack manifest. A caller may
  supply a replacement registry or an explicit exact identity for new-build
  research. Original bytes and host paths are not copied into either output.
- `identify-orig` classifies a scanned tree through a canonical registry of
  complete fingerprints. The binary bundles the audited GZ2E01 GameCube USA
  identity; unknown bytes remain explicitly unsupported, while a requested
  friendly ID whose fingerprint disagrees is rejected.
- `cache-fact-pack` installs a verified payload/manifest pair in the planner's
  immutable manifest-digest store; identical installs are reused.
- `materialize-fact-pack` retrieves and re-verifies that derived pack without
  needing the original game assets.
- `list-archive-resources` emits sorted file basenames from a bounded Yaz0/RARC
  archive so resource-discovery exceptions can be audited instead of guessed
  from archive filenames.
- `extract-message-flow` emits the BMG flow labels, nodes, branch-target table,
  correctly resolved query-handler numbers, raw 32-bit event parameters, and
  source-derived temporary-bit, persistent-event-bit, and switch accesses. Raw
  query-table indices remain present so the two coordinate systems cannot be
  conflated.
- `extract-stage-data` emits planner-owned DZS/DZR chunk records, STAG message
  groups, indexed SCLS destinations, REVT event/exit coordinates, LBNK demo
  archive selections, and authored actor placements including layer, parameters,
  position, rotations, and raw bytes. Unknown chunk formats remain listed but
  uninterpreted.
- `extract-event-list` emits the bounded event, staff, cut, linked parameter,
  and typed value tables from one exact `event_list.dat`, retaining raw records
  and rejecting invalid references or overlapping tables.
- `extract-function-evidence` resolves one exact text-function record from a
  symbol table into its DOL text section, seals both source identities and the
  selected bytes, and classifies only the mechanically decidable one-instruction
  `blr` shape as an immediate return. Gameplay meaning remains a separate
  audited binding.
- `extract-binary-range-evidence` resolves a bounded virtual-address range
  through exactly one loadable DOL text or data section and seals the selected
  bytes without assigning them semantic meaning. Zero, oversized,
  cross-section, overlapping-section, and truncated ranges fail closed.
- `extract-jstudio-stb` performs a bounded structural decode of one JStudio STB
  from a supplied archive. It indexes embedded FVB functions and decodes object
  IDs, sequence commands, waits/suspends/jumps, and paragraph boundaries while
  hashing rather than embedding animation/camera payloads. Object-specific
  paragraph effects remain explicitly unresolved.
- `extract-demo-actor-program` exact-binds GZ2E01 `d_actN` objects to the retail
  generic demo actor and decodes type-`0x80` status-51 packed command words. It
  identifies persistent and temporary event-bit writes without claiming that
  the authored actor was successfully created or executed.
- `resolve-jstudio-stb` applies an exact-content adaptor profile to those
  paragraphs, decoding proven variable/adaptor payload contracts and retaining
  unsupported dispatches as unresolved records.
- `resolve-cutscene-package` joins an exact wrapper and nominal STB semantics to
  a build-scoped archive/PACKAGE runtime profile without promoting an unknown
  corruption producer or outer event exit.
- `resolve-cutscene-outer` verifies exact stage/event-list resources, derives
  PACKAGE PLAY-to-WAIT completion flags, and emits normal, skip, and suppressed
  exact-context candidate transitions without choosing unknown runtime flags.
  Transition identities and destination labels derive from the selected event
  and exits; the resolver is not hard-coded to the downstream Zelda cutscene.
- `compile-cutscene-corruption-hypothesis` emits an unknown-evidence producer
  for only the named failed-load predicate. It carries explicit failure-site,
  predicate, and prefix unknowns and cannot directly change location or return
  place. The producer binds to the exact outer event supplied by the caller.
- `extract-cutscene-wrapper` joins one named REVT/LBNK/SCLS event to its exact
  `event_list.dat` staff/cut/parameter graph. Its coverage record keeps JStudio
  phases, exceptional load flow, and return-place writers unresolved until a
  separate decoder or trace establishes them.
- `inspect-state` exposes every live and serialized component store alongside
  exact-context friendly aliases and derived fact evaluations. It also retains
  ordered operation/boundary history, reports the last known writer of each live
  structured field, and groups set/clear history for every observed write gate.
- `project-graph` emits a planner-native causal graph with typed relations and
  collapsible predicate regions and optional route-book plan regions; it does
  not use TAS timeline graph schemas.
- `project-feasibility-diff` evaluates each candidate at one exact executable
  state and emits only edges whose permissive upper-bound and modeled results
  differ, including obstruction, obligation, and microtrace proof details.
- `state-from-snapshot` materializes an executable planner state.
- `validate-route-book` checks a route book's goals, predicates, action
  references, nested regions, methods, directives, and annotations against an
  exact base or composed catalog without adding mechanics.
- `solve` runs bounded causal reachability against planner-owned catalogs and
  can apply an optional route book's scoped constraints, action directives,
  ordered conditioned methods, cost/evidence thresholds, and deterministic soft
  preferences. Reached steps retain obstruction/resolver/obligation choices;
  failed searches retain a deterministic closest blocker witness per transition.
  Predicate-shaped physical obligations are recomputed from each propagated
  state, so a state write can unlock a transition without a named route shortcut.
  Gated writer rules execute as their own searchable action type; transition
  proofs retain attached reader values, and missing in-scope readers fail
  unknown instead of being skipped.
  Catalog-goal solves first expand backward from the goal through all typed
  producers and requirements, then restrict forward exploration to that causal
  slice. The solve report retains the full relevance frontier and whether
  pruning was enabled. Required route predicates, pinned actions, and required
  method steps with their pre/postconditions become additional roots, so route
  authoring remains enforceable without retaining unrelated mechanics.
  Physical obligations can derive required/excluded box, sphere, or cylinder
  membership; loaded-actor state; directed region connectivity; plane sidedness;
  player rotation, action, and control; and evidence-scoped temporal microtraces.
  Absent actors, geometry, or timing witnesses remain unknown. Exact matching
  temporal witnesses appear in solve proofs and auto-bind to their obligations
  in the planner graph. Composed solve reports retain each active refinement
  entry's layer, pack ID, digest, and local precedence. Reached steps and blocked
  witnesses embed all contributing rule/fact evidence and report their weakest
  evidence level, so hypothetical support cannot be hidden behind an established
  transition label.
- `solve-portable` expands a route book's exact/equivalent context scope,
  requires one explicit start state per exact context, solves each context
  independently, and reports whether the route reaches its goal everywhere.
- `serve-stdio` exposes typed validate/compose/project/solve requests as JSON
  lines for a future planner editor or other clients, including portable solves.

The low-level extraction commands are read-only with respect to `orig/`.
They write only the explicitly named output and record SHA-256 identities for
both the source archive and extracted resource. They do not call or link
Huntctl/TAS tooling.

Stage-local backing is also planner-owned. A `StageBank` address contains both
the runtime-file ID and stage name; it is never a process-global “Forest Temple
bank.” Mechanics can use `commit_load_stage_bank` to atomically verify and
commit the live source payload, restore the exact destination payload, and apply
explicit source/destination semantic bindings. Scene movement stays a separate
effect, preventing a bank swap from silently inventing a map transition. See
`docs/route-planner/backing-store-boundaries.md`.

Physical saves use a second level of ownership. A populated slot points to a
digest-sealed persistent-file image containing an explicit runtime-component
manifest and its nested stage-bank stores. `save_runtime_to_slot` creates or
overwrites that image without pretending file 0 became slot 0;
`load_runtime_from_slot` restores it into a fresh card-backed runtime identity,
records the prior runtime as ended, and leaves session-owned components outside
the projection. `activate_stage_bank` performs the initial stage-bank restore,
while `set_location` remains a separate effect.

Raw stage-memory semantics can also be addressed by backing rather than by a
transient component ID. `bound_raw_bits` reads numeric fields or item masks from
one exact component kind/binding, and `adjust_bound_raw_unsigned` mutates a
uniquely bound known count atomically. This is how small-key counts and dungeon
items follow the active stage bank through ordinary loads or explicit rebinds;
see `docs/route-planner/bound-stage-memory-semantics.md`.

Masked `write_bound_raw` and `invalidate_bound_raw` operations address the same
runtime-resolved backing references. Imported flag and switch writers therefore
follow the active runtime file, stage, or room without hard-coding a transient
component ID.
