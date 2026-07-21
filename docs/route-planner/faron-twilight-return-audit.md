# Faron-twilight return audit (GZ2E01)

This audit asks which known or conceivable mechanisms can reach the following
maps while the persistent Faron-dark-clear flag is still false. It deliberately
separates an encoded destination from an executable route. An `SCLS` record,
cutscene destination, or stored return place is only a producer of a possible
scene change; its actor, event, geometry, form, and backing-store predicates
still have to be established.

## Target identities

| Friendly target | Scene identity used by the planner |
| --- | --- |
| Goats map / Ordon Ranch | `F_SP00`, room 0 |
| Ordon Village | `F_SP103`, room 0 |
| Outside Link's house | `F_SP103`, room 1 |
| Link's house | `R_SP01`, room 4 |
| Ordon Spring | `F_SP104`, room 1 |

The layer is derived after entry. In particular, the game selects pre-clear
layers for `F_SP00`, both rooms of `F_SP103`, and room 1 of `F_SP104` while
Faron twilight is uncleared; it does not follow that the corresponding entrance
is reachable. See the stage-specific layer selection in
[`d_com_inf_game.cpp`](../../src/d/d_com_inf_game.cpp).

## Executability conclusions

The currently established route family is a Midna portal warp to Ordon Spring,
followed by ordinary local room transitions. The decoded GZ2E01 field-map portal
entry 0 targets `F_SP104`, room 1, point 0 and is unlocked by stage 0 switch
`0x34`. Executing it additionally requires the first-portal-warp event, an
unlocked portal entry, and Link accepting dungeon warp; those are independent
guards in [`d_menu_fmap2D.cpp`](../../src/d/d_menu_fmap2D.cpp).

From Ordon Spring, wolf Link can use the crawlspace to the yard outside Link's
house. The yard connects to the main village, which connects to the ranch. Link's
house itself has an extra form obstruction: the `kdoor` actor only offers its
door event while Link is not a wolf, as shown in
[`d_a_door_knob00.cpp`](../../src/d/actor/d_a_door_knob00.cpp). Thus the house
route needs a separately established human-in-twilight producer, such as the
EMS state, and its ordering matters: use the wolf-only crawlspace before
changing to human, then open the knob door.

The direct walk from the Faron side remains blocked by the active twilight
barrier. The route model retains the encoded exit but attaches a scoped
`cross active Faron twilight barrier` obligation and reports the barrier as the
active obstruction. A proposed wolf OOB route is retained as feasibility
unknown rather than silently accepted. Epona OOB is an established technique
class but is inapplicable in this state because it needs Epona and a non-twilight
setup.

## Warp, reload, and wrong-state candidates

| Mechanism | What supplies the destination | Result in the audited start state |
| --- | --- | --- |
| Midna portal | Field-map portal table | Established route to Ordon Spring when the first-warp, switch `0x34`, and player-acceptance guards hold. |
| Savewarp | `PlayerReturnPlace`/return-stage backing | Established consumer, but the audited file holds `F_SP108`; it reaches Ordon Spring only if an earlier writer actually left `F_SP104`. |
| Void reload | Restart/respawn backing | Same distinction: executable only when the live restart component contains an Ordon target. |
| Death reload | Restart/respawn backing | Same as void, subject to its own death transition and reconstruction behavior. |
| Title/load | Selected physical save slot | Can load an Ordon-positioned file, but swaps runtime-file identity; retaining the desired twilight state is a predicate of that slot, not a free effect. |
| Normal BiT save/load | File-0 save projection and subsequent load | Returns to `F_SP108` (Faron Spring), not `F_SP104` (Ordon Spring). It is surfaced as a candidate but does not solve an Ordon goal. Saving also projects slotless file 0 into slot 1, 2, or 3 rather than making file 0 saveable. |
| BiTE | Compatible existing-file selection plus transferred equipped/runtime state | Can enter a compatible Ordon-positioned file, but ends the active file-0 lifetime. Compatibility and the King Bulblin/room setup must be proven separately. |
| Held return-place transfer | Proposed component preservation/rebind | A valid research overlay: if it writes or transfers `F_SP104` into the exact return-place component, the unchanged savewarp reader can consume it. It remains hypothetical unless a transfer mechanism is witnessed. |
| Wrong-state respawn/spawn injection | Proposed restart or spawn mutation | Retained as hypothetical and scoped; absent a writer, it is not an executable transition. |
| Cutscene scene change | Cutscene/event `SCLS` consumer | Candidate only. The relevant event activation and skipped-cutscene postconditions are not yet witnessed. |
| Actor-driven scene change | Actor parameters plus collision/event activation | Candidate only. An encoded target does not prove a reachable activation volume. |
| Resource-load failure/actor corruption | Failure path around cutscene or actor creation | Candidate class only. It may preserve an earlier save location, but exact skipped writers and cleanup effects need a trace or code proof. |

This is intentionally not represented as a `loses: Ordon` annotation. The
planner reads the actual location, return-place, restart, runtime-file, form,
mount, control, portal, and story components at each step. A new writer or
transfer technique can therefore make an existing consumer usable without
editing a hard-coded lockout list.

## Exhaustive encoded incoming-destination scan

Every GZ2E01 room archive under `orig/GZ2E01/files/res/Stage` was decompressed
and scanned for `SCLS` records naming one of the five target scenes. The table
groups records by source archive. Record numbers are zero-based. Repeated and
self-targeting records are retained because different actors, exits, and event
branches may consume them.

| Source room | Matching record(s) and encoded destination |
| --- | --- |
| `D_SB01/R09` | 1 -> `F_SP104/R01` |
| `D_SB01/R49` | 0 -> `F_SP104/R01` |
| `D_SB05/R00` | 2 -> `F_SP00/R00` |
| `F_SP00/R00` | 0,2 -> `F_SP103/R00`; 4 -> `F_SP103/R01`; 1,3,6,7,8,9,10,12 -> `F_SP00/R00` |
| `F_SP103/R00` | 0 -> `F_SP103/R01`; 1,3,11 -> `F_SP00/R00`; 12,20,21 -> `F_SP103/R00` |
| `F_SP103/R01` | 0,8,14 -> `F_SP103/R00`; 1,7 -> `F_SP104/R01`; 2,4,5,6 -> `R_SP01/R04`; 3,9,10,11,12,13,16 -> `F_SP103/R01` |
| `F_SP104/R01` | 0,9,10,15 -> `F_SP103/R01`; 3,6,16,17,18 -> `F_SP104/R01` |
| `F_SP108/R00` | 0,5 -> `F_SP104/R01` |
| `F_SP108/R01` | 0,4 -> `F_SP104/R01`; 2 -> `F_SP103/R01` |
| `F_SP108/R03` | 1 -> `F_SP00/R00` |
| `F_SP108/R04` | 3 -> `F_SP103/R00` |
| `F_SP108/R06` | 6 -> `F_SP00/R00` |
| `F_SP117/R01` | 8 -> `F_SP103/R01` |
| `F_SP121/R10` | 0,4 -> `F_SP104/R01` |
| `F_SP200/R00` | 2 -> `F_SP104/R01` |
| `R_SP01/R00` | 0,1 -> `F_SP103/R00` |
| `R_SP01/R01` | 0 -> `F_SP103/R00` |
| `R_SP01/R02` | 0,1 -> `F_SP103/R00` |
| `R_SP01/R04` | 0,2,3,4,5 -> `F_SP103/R01` |
| `R_SP01/R05` | 0,1 -> `F_SP103/R00` |
| `R_SP107/R02` | 3,4 -> `F_SP104/R01` |
| `R_SP107/R03` | 6 -> `F_SP104/R01` |
| `R_SP109/R00` | 10 -> `F_SP104/R01` |
| `R_SP160/R01` | 36 -> `F_SP00/R00` |

This scan is an upper bound, not a route list. Before any row becomes an edge,
the planner still needs the consumer binding (exit, door, actor, cutscene, or
event branch), its activation predicate, collision reachability, and witnessed
postconditions. The unusual dungeon/interior sources are especially likely to
be event, demo, or fallback records rather than player-walkable exits.

## Planner acceptance fixture

The engine fixture
`faron_twilight_return_audit_keeps_warps_blockers_and_file_lifetimes_distinct`
models all five goals and checks:

- the portal/local-transition route reaches each target without clearing Faron
  twilight;
- Link's house requires both EMS-provided human form and the knob-door edge;
- normal BiT save/load is relevant to the broad location search but is not used
  as an Ordon solution;
- with the portal switch closed, direct walking is reported obstructed and the
  wolf OOB proposal is reported feasibility-unknown with its exact missing
  obligation;
- research mode can compose a hypothetical return-place writer with the normal
  savewarp reader, and the route proof retains hypothetical evidence.

Remaining work is evidentiary rather than a reason to flatten the graph: bind
each raw `SCLS` record to its actual consumer; witness cutscene, resource-failure,
and actor-driven paths; identify exact restart/save writers; and repeat the
archive audit for every supported build.
