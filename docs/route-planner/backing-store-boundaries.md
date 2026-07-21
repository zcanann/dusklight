# Backing-store boundaries

The planner represents a live component, its semantic binding, and its physical
serialization owner independently. For the normal `dSv_info_c::putSave(stage)`
then `getSave(stage)` flow, a stored stage-bank address is:

```text
StageBank(runtime_file_id, stage)
```

The runtime-file coordinate is required. Without it, file 0 and a loaded card
file would incorrectly share one stored Forest Temple payload.

## Normal stage-bank operation

`commit_load_stage_bank` is an ordinary executable state operation with these
inputs:

- live component ID;
- active runtime-file ID;
- source and destination stage names; and
- explicit source and destination semantic bindings.

Application is transactional. It succeeds only when:

- the named runtime file is active;
- the current scene is the source stage;
- the live component has stage-load lifetime, the authored source binding, and
  owner `StageBank(runtime_file_id, source_stage)`;
- `StageBank(runtime_file_id, destination_stage)` contains the same component
  identity and kind with the authored destination binding and stage-load
  lifetime.

The engine first copies the live payload into the exact source store, then
restores the destination store into the live component. Both copies receive
transition provenance. If any check or destination lookup fails, neither the
live component nor either backing store changes.

The operation does not change scene location. A normal transition authors a
separate `set_location` effect after the bank operation. This keeps storage
semantics distinct from authorization, collision, doors, cutscenes, and other
ways the map transition itself may be blocked or bypassed.

Every serialized component must name the same owner as the store containing it.
A stage-bank store additionally accepts only stage-load-lifetime components.
These invariants are checked when an execution state is created, decoded, or
committed, so malformed theorycraft data cannot silently alias another store.

## Binding is not ownership

The source and destination bindings are explicit rather than inferred from the
stage-bank key. A payload may be interpreted through a stage, dungeon, zone, or
other evidenced binding while still residing at a runtime-file-scoped stage
owner. Theorycraft rebinding can therefore alter semantic interpretation without
moving bytes, while a normal bank operation moves bytes between owners without
guessing their meaning.

The current operation requires the destination entry to exist. Extraction or an
initializer must seed first-entry payloads; absence is an execution failure, not
an implicit zero-filled bank. Physical-slot save/load and title/file lifecycle
programs operate over a second owner level.

## Persistent file images and physical slots

A populated physical slot does not directly own a flat component list. It names
one canonical `persistent-file-image/v1` containing:

- an explicit, sorted set of runtime-file components selected by the save
  policy; and
- explicit, sorted stage-bank stores whose owners are rekeyed to the persistent
  file identity.

The slot stores the image digest. Execution-state validation requires a
one-to-one correspondence between populated slots and images and re-verifies
every digest. Two physical slots cannot silently name one persistent identity.

`save_runtime_to_slot` checks that the source is active and the destination is
one of its allowed slots. It commits live components belonging to selected
stage banks, projects only the authored runtime-component/stage manifest, marks
the copies with save/restore provenance, and atomically installs the sealed
image. The source runtime stays active; this operation alone does not claim that
file 0 ended or that title/void preconditions were satisfied.

`load_runtime_from_slot` requires its authored component and stage lists to
match the sealed image exactly. It then:

1. removes only live and stored components owned by the source runtime;
2. retains unrelated session/process components;
3. restores the image under a fresh runtime-file identity;
4. records the source runtime with `ended` lifecycle; and
5. activates the fresh runtime with `loaded_slot` origin, card backing, and the
   explicitly authored set of future save targets.

`activate_stage_bank` is the initial `getSave(stage)` half: it restores one
loaded stage entry into an absent live component without committing a previous
stage. A following `set_location` chooses the scene. When all operations are in
one transition batch, a missing image, incomplete manifest, component collision,
missing stage bank, or bad location rolls the entire batch back.

The exact file-0 initial image, void/title prerequisites, and build-specific
save normalization/clearing remain evidence tasks. The mechanism deliberately
does not invent them.

## Schema revision

Adding the runtime-file coordinate is a wire-format change, not a friendly-label
reinterpretation. The milestone advances the directly containing schemas:

- execution environment and state snapshot/diff/chain to v6;
- execution state to v8 and boundary policy remains v2;
- fact catalog to v5, mechanics catalog to v14, refinement pack to v13, and
  composed catalog to v14;
- route book/edit batch to v5, graph projection to v7, cutscene program/compiled
  program to v6, and extracted world facts to v6; and
- state inspection to v8, inspection diff to v7, and planner service to v20.

Canonical decoders fail closed on prior shapes. They do not synthesize a
runtime-file ID for an old stage-only owner, because doing so could merge stores
that were physically distinct.
