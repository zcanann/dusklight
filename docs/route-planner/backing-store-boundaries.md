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
programs remain separate tasks because they operate over different owners and
reset domains.

## Schema revision

Adding the runtime-file coordinate is a wire-format change, not a friendly-label
reinterpretation. The milestone advances the directly containing schemas:

- execution environment and state snapshot/diff/chain to v5;
- execution state to v6 and boundary policy to v2;
- mechanics catalog to v10, refinement pack to v9, and composed catalog to v10;
- cutscene program/compiled program to v2 and extracted world facts to v2; and
- state inspection to v6, inspection diff to v4, and planner service to v15.

Canonical decoders fail closed on prior shapes. They do not synthesize a
runtime-file ID for an old stage-only owner, because doing so could merge stores
that were physically distinct.
