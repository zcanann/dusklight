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

The active-runtime form may additionally carry a bounded ordered projection
program. The executor applies only projection-safe field/raw writes to a private
clone, seals the image from that clone, and copies back only the physical-slot
and image stores. The live runtime and its stage banks are therefore untouched.
Every projection target must belong to the explicit runtime-component manifest;
nested saves, ownership changes, lifecycle changes, and writes to session-only
targets fail catalog validation. GZ2E01 `memory_to_card` uses this boundary for
its temporary monkey-event and lantern/oil adjustments.

`load_runtime_from_slot` requires its authored component and stage lists to
match the sealed image exactly. It then:

1. removes only live and stored components owned by the source runtime;
2. retains unrelated session/process components;
3. restores the image under a fresh runtime-file identity;
4. records the source runtime with `ended` lifecycle; and
5. activates the fresh runtime with `loaded_slot` origin, card backing, and the
   explicitly authored set of future save targets.

The load operation also accepts an explicit runtime-component carry manifest.
This manifest is separate from the persistent-image manifest: carried state is
not relabeled as card data. Every carried ID must name a live, runtime-lifetime
component owned by the source runtime, cannot name a stage-bank/physical-slot
component, and must be disjoint from the exact card-image component IDs. Those
selected components are rekeyed to the fresh destination runtime with transition
provenance. Unselected source-runtime metadata and every source-owned serialized
store are removed. A missing, duplicate, unsorted, card-overlapping, session,
stage-bank, or otherwise invalid carry entry fails the whole load atomically.
This is the generic splice required for BiTE-like preservation; which concrete
components a retail BiTE setup carries remains an evidence-matrix task.
The source-audited GZ2E01 file-select transitions also use an active-runtime
form of the load. It resolves the persistent identity and complete sealed
manifest from the selected populated slot, derives the destination runtime ID
from the actual active lifetime plus a stable suffix, and then applies the same
checked load. This avoids embedding ephemeral runtime/persistent IDs or partial
stage manifests in authored mechanics.

Process buffers use separate custom owner stores. `replace_custom_store`
installs one exact, sorted component manifest without touching runtime or
physical stores. `restore_payloads_from_custom_store` requires its authored IDs
to match that store exactly, copies each payload into a same-ID/same-kind live
component, propagates source provenance, and retains the destination's live
binding/lifetime/owner. GZ2E01 no-card selection uses three such session stores;
none populates a physical slot.

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
- execution state to v9 and boundary policy remains v2;
- fact catalog to v5, mechanics catalog to v15, refinement pack to v14, and
  composed catalog to v15;
- route book/edit batch to v5, graph projection to v7, cutscene program/compiled
  program to v7, and extracted world facts to v7; and
- state inspection to v9, inspection diff to v8, and planner service to v21.

Canonical decoders fail closed on prior shapes. They do not synthesize a
runtime-file ID for an old stage-only owner, because doing so could merge stores
that were physically distinct.

The later explicit process-context milestone advances execution environment to
v7, state snapshot/diff to v8/v7, execution state to v10, fact catalog to v7,
mechanics catalog to v17, state inspection/diff to v10/v9, and planner service
to v22.

The subsequent file-0 initializer advances execution state to v11, fact catalog
to v8, mechanics catalog to v18, state-inspection diff to v10, and planner
service to v23. Whole-payload invalidation can include only serialized stores
owned by the active runtime; inactive runtime stores and physical images remain
separate storage sites.

The title-file-0 lifetime handoff advances execution state to v12, mechanics
catalog to v19, and planner service to v24. `begin_runtime_file_lifetime` ends
the incoming active lifetime, derives a fresh runtime ID, and rekeys only that
lifetime's live and serialized owners. It does not reinterpret a card image as
the live file, invent slot 0, or absorb session state.
See `gz2e01-title-boundary-audit.md`; the versions above record the earlier
runtime-file-coordinate milestone rather than the current wire versions.

The selected runtime-component carry manifest advances execution state to v13
and mechanics catalog to v20. The lifetime-cut inspection advances
state-inspection diff to v11 and, together with the carry operation, planner
service to v26. A lifetime-cut report is derived from two executable states: it
classifies every source-owned live component and serialized store by its actual
destination payload/ownership fate, then separately reports unchanged/changed
outside-lifetime live components and sealed physical-file images. It does not
encode a game-specific list of alleged BiT losses.

The reconciled file-select milestone advances execution state to v15, fact
catalog to v9, mechanics catalog to v25, and planner service to v28. Custom buffer
stores and derived active-runtime loads are generic operations; their GZ2E01 use
remains scoped to the exact source-audited context. Exact DOL function evidence
binds `card_to_memory` and its lineup helper. Typed operations now execute the
conditional 12-life floor, dungeon-6 key reset, parameterized hookshot migration
and lineup rebuild, saved vibration application, and return-place-derived
display-stage update. Physical
slot availability is a three-valued state reference: verified image presence is
true, an explicit absent observation is false, and uncaptured/not-sampled state
remains unknown. A structured-field operation can attach a return-place-derived
pending world load to the active process without activating that world.

The successful-save milestone advanced mechanics catalog to v26.
`save_active_runtime_to_slot` derives the persistent image identity from the
runtime that is active at execution, selects every available stage bank owned
by that runtime, and delegates to the existing atomic slot projection. This
keeps authored save-menu mechanics valid across prior load/lifetime handoffs
without weakening slot authorization or treating a failed asynchronous command
as a sealed image.

The transformed-save milestone advances mechanics catalog to v27. The same
operation now accepts the validated private-clone projection program described
above, and byte-valued structured fields support masked reads/writes without
being relabeled as opaque raw components.
