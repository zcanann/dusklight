# Auru recent-item store and boundary audit

This audit isolates the generic item-presentation state used by Auru duplication
from Auru's actor logic and from build-specific interaction geometry.

## Physical owner and direct writers

`dEvt_control_c` is embedded in the process-wide `dComIfG_play_c` object. Its
adjacent byte fields are:

| Offset | Field | Meaning |
| --- | --- | --- |
| `0x0ee` | `mPreItemNo` | item offered to the current show-item/talk/catch event |
| `0x0ef` | `mGtItm` | most recently created presentation/get-item demo item |

A whole-source search finds exactly two calls to `dComIfGp_event_setGtItm`, both
in `src/f_op/f_op_actor_mng.cpp`:

1. `fopAcM_createItemForPresentDemo(...)` writes its `i_itemNo` before creating
   a presentation item actor.
2. `fopAcM_createItemForTrBoxDemo(...)` writes its `i_itemNo` before creating a
   treasure/get-demo item actor.

In the audited source tree, 48 call sites in 38 other files use the first writer
and 12 call sites in 12 other files use the second. NPC rewards, shops, Poe
souls, ordinary chests, small keys, dungeon items, insects, and loose scripted
items therefore share one last-writer-wins byte. Provenance identifies which
presentation wrote the value; provenance does not change the byte's semantics.

The write occurs before either helper rejects `dItemNo_NONE_e`, so even a failed
creation request has already overwritten `mGtItm`. Actor-creation failure after
a valid request likewise does not roll the write back.

## Things that do not write `mGtItm`

`SHOWITEM_X`/`SHOWITEM_Y` and catch-event setup write `mPreItemNo`, not
`mGtItm`. `dEvt_control_c::endProc()` and `remove()` clear `mPreItemNo`; neither
touches `mGtItm`. The two `reset()` overloads only request/change event cleanup
and also do not touch it.

This distinction is essential for text displacement and Auru theorycrafting. A
shown inventory item can change the pending dialogue branch without changing
the recent presentation item, while a newly created presentation can replace
the item later consumed by a generic get-item cut.

## Consumer and self-reassertion

In `daAlink_c::procGetItemInit()`, a nonzero demo parameter normally names an
item directly. Parameter `0x100` instead reads `dComIfGp_event_getGtItm()`.
Link then calls `fopAcM_createItemForPresentDemo` with the resolved ID, so the
generic consumer reasserts the same value through the ordinary writer before
granting it. It does not consume or clear the byte.

Auru's normal memo path creates the memo presentation first and therefore
overwrites the byte with item `0x90`. The interrupted/broken path reaches the
generic `0x100` consumer without that overwrite, so whatever compatible
presentation ran most recently remains the selected item.

## Boundary matrix

`dComIfG_play_c::init()` resets player/camera pointers and game-over state but
does not reconstruct or clear its embedded event controller. Event-controller
completion, reset, and removal do not clear `mGtItm`. Save data loading replaces
`dSv_info_c`/runtime-file state, not the process-owned play object. The modeled
result is:

| Boundary | `mGtItm` | Reason |
| --- | --- | --- |
| Dialogue/event cleanup | preserved | only `mPreItemNo` is cleared |
| Room or stage transition | preserved | play/event controller remains process-owned |
| Void/death-style reload or savewarp | preserved | scene/file state changes do not rewrite the byte |
| Load another physical save slot | preserved | save-domain data is replaced; play session is not |
| Save runtime file to a slot | preserved, not serialized | owner is process session, not runtime file/card |
| Return through title within the same process | preserved | soft initialization/removal has no writer |
| Wrong-state respawn | preserved | no implicit writer |
| Any later presentation/chest request | overwritten | the shared helpers are unconditional writers |
| Fresh process/global construction | reinitialized | static process storage starts anew |

The planner acceptance test applies every modeled in-process boundary to the
session component, separately proves event cleanup clears `mPreItemNo` without
touching `mGtItm`, proves later presentations overwrite it, and reserves an
explicit process-restart boundary for reinitialization. No ordinary boundary is
allowed to infer persistence merely from a friendly “Auru duplication” label.

## Scope and remaining evidence

This is source evidence for the shared GCN code in this repository. It explains
why the mechanic can be represented for SD even when no feasible SD interaction
setup is known. TPHD targeting/sidehop feasibility remains external evidence,
and exact fresh-process initial values for other executables require their own
binary/content identity rather than silent universality.
