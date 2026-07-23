# GZ2E01 standard BiTE preservation matrix

The engine-owned `bite-preservation-matrix/v1` artifact records the conditional
state boundary for the standard selected-file form of Back in Time Equipped.
It is exact to GZ2E01 with the English runtime configuration and is sealed over
canonical JSON. It does not treat “BiTE” as a primitive whole-file teleport.

## Matrix semantics

The matrix builder derives its component manifests from the executable title
mechanics rather than maintaining a second handwritten transfer list:

| Disposition | Exact subjects | Meaning |
| --- | --- | --- |
| `restored_from_selected_file` | `flags.persistent-event-registers`, `inventory-and-resources`, `return-place`, `save.dungeon-memory.index-6`, `save.player-info`, `save.player-light-drop` | The values come from the sealed selected-file image. They are not title/BiT values carried into that file. |
| `carried_from_bit_runtime` | `flags.temporary-event-registers`, `restart`, `runtime-file.header` | The checked active-runtime load explicitly rekeys these non-card runtime components into the new loaded-file lifetime. |
| `removed_with_bit_runtime` | `source-runtime.omitted-components` | Every other component and serialized store owned by the ended source runtime is removed. |
| `preserved_outside_runtime_lifetime` | `outside-runtime.session-components` | Session/process state survives because it is outside the runtime-file ownership cut, not because it was saved or carried. |
| `unchanged_sealed_backing` | `physical-slot.unselected-images` | Loading the selected slot does not rewrite other sealed slot images. |

The selected-file projection is the source-audited `dSv_save_c` save/load
boundary. Consequently, visible items, equipment, and progress are classified
as selected-file restoration. They are explicitly not classified as carried
title-Link properties.

## Community evidence and exact unknowns

The ZeldaSpeedRuns standard BiTE documentation reports that the selected save's
items, equipment, and progress appear after the load, that standard BiTE
replaces title-Link properties, and that the save needs a compatible Eldin
field room-0 spawn. It also records the King Bulblin fight as a witnessed
outcome:

<https://www.zeldaspeedruns.com/tp/bit/back-in-time-equipped>

Those reports establish the compatibility and visible-state rows, but they do
not identify the exact native overlap writer, timing program, or destination
splice in the registered GZ2E01 binary. The matrix therefore keeps
`activation.standard-bite-overlap` and `destination.king-bulblin-fight` at
`unknown`. The artifact can be used as the effect contract once a setup
transition discharges those activation obligations; it cannot authorize the
setup by itself.

The death-method variant described by the same source retains title-Link
properties and is a different boundary. It is excluded from this standard
selected-file matrix rather than being merged into a contradictory carry list.

## Validation boundary

Validation fails closed unless the artifact contains the complete sorted
subject/disposition set, the exact context selector, coherent evidence truth,
and a matching content seal. Canonical decoding also rejects alternate JSON
encodings. Unit tests independently confirm the mechanics-derived save/carry
sets, ensure equipment is never mislabeled as runtime carry, preserve the two
native unknowns, reject tampering, and reject other content contexts.
