# GZ2E01 representative actor-state inventory

This inventory separates the three state domains for representative exact-build
door and gate families. It is a routing index over the family-specific audits;
it does not make their layouts interchangeable.

## Domain rule

- **Static placement** is immutable content selected by exact build, archive,
  stage, room, layer, actor name, parameters, transform, and home angles.
- **Persistent control** is save/runtime state that reconstructs an actor after
  load. A switch, event bit, key count, or boss-key bit belongs to its exact
  component binding; it is not a live animation or collision flag.
- **Transient instance state** exists only while the actor/process is live:
  offer geometry, event/action phase, keyhole or child relationship, animation,
  collision registration, latches, paired actors, and pending key deltas.

The planner's `StaticWorldObject`, `PersistedObjectControl`, and
`LiveWorldObject` records correspond to these domains. Reconstruction consumes
static placement plus persistent control to create live state. It never derives
persistent state from a live pose or assumes that a persistent unlock means an
actor is currently open.

Mechanics catalog v29 makes that reconstruction executable as one atomic state
operation. It requires active world execution, an exact static placement bound
to the current stage/room, the authored layer, and either no prior live instance
or one whose lifecycle is `unloaded` or `destroyed`. The resulting loaded actor
starts with placement parameters, overlays the matching persisted-control
fields, and finally overlays the rule's audited initialization fields. A wrong
room/layer, absent placement, mismatched instance identity/type, or already
loaded/unloading instance rejects the whole operation without a partial actor.
Reconstruction rules remain instantiation-boundary evidence and are not exposed
as freely selectable solver actions.

## Audited representatives

| Family | Static placement | Persistent control | Transient instance state | Reload reconstruction |
| --- | --- | --- | --- | --- |
| `DOOR20` / `yodoor` | Door kind; front/back options and rooms; unlock/secondary switches; position and angles | Current-stage `dSv_memBit_c` unlock switch and small-key byte | Usable side; event variant/action; lock/key type; keyhole child; pending decrement; animation; opening/closing flags; realized angle; background collision; unlock-effect latch; stopper/enemy-clear state | Switch clear creates a locked closed door and keyhole; switch set creates an unlocked closed collision-registered door without a keyhole |
| L1 boss-door family | Actor-family name; front/back rooms; SCLS exit; unlock switch; transform | Current-stage boss-key bit and memory-switch unlock | Usable side; actor-local interaction test; offer/event phase; keyhole; animation; collision; scene-change and restart phases | Persistent switch controls first-unlock/keyhole behavior; traversal still requires a new live open sequence |
| L5 boss door | Stage-specific actor placement; front/back rooms; exit; switch; transform | Human form is live player state; boss-key and dungeon-side unlock switch are current-stage control | Positive-local-Z side; interaction box/facing; optional keyhole by stage type; offer/unlock/open/collision/scene-change phases | Dungeon-side stage type may reconstruct a keyhole while boss-room stage type suppresses it; the reverse placement must not inherit the dungeon-side switch write |
| Keyed mini-boss doors | Front/back options and rooms; exit; primary and alternate switches | Current-stage key count and memory switch | Front-side selection; event staff; queued decrement; unlock/open/collision/scene-change phases | Set switch suppresses another decrement but does not prove an active open phase |
| `kshtr00` / `L3Bdoor` key shutters | Runtime type, event selector, check-key bit, switch, placement | Small-key or boss-key guard plus memory switch | Event phase, queued decrement, animation and background collision | Set switch reconstructs the shutter open with collision released; the separate `vshuter` consumer opens from current-room one-zone switch `0xef` without an internal key guard or writer |
| `K_Gate` | Type, switch, layer, pose | Current-stage memory switch and key count | Facing/local box; pending decrement; leaf angles; player/horse/coach contact | Set or absent switch initializes the type-0 gate fully open and disables further gate action |
| `R_Gate` | Switch, layer, pose | Key count, memory or dungeon-session switch domain, and persistent event-bit bypass | Interaction box, pending decrement, paired leaf motion | Event bit may force open independently of the switch; F_SP121 uses the observed dungeon-session view rather than stage memory |
| `CrvGate` | Paired room placements and relationship | No ordinary persistent unlock switch; key count and destructive bypass event are separate controls | Parent/child pair, accepted key event, transient opening, boar collision/destruction | Reload cannot infer a permanent unlock from an ordinary transient key opening |
| `L7demo_dr` bridge demo | Placement, local box, exit and switch | Key-count guard and memory switch; no key consumption | Cutscene/event and scene-change phases | Key presence authorizes the demo but is not consumed; this is not a keyed door |

## Timing and ownership boundaries

The accepted unlock cut and committed persistent outcome need not share one
update. `dComIfGp_setItemKeyNumCount(-1)` queues a transient delta;
`dMeter2_c::moveKey()` later updates the currently bound `dSv_memBit_c` key byte
and clears the queue. The unlock switch may already be set between those events.
The inventory therefore keeps the queue in live/process state and the committed
count in stage memory.

Likewise, collision is live actor state. `DOOR20` reloads an unlocked door as
closed with collision registered; keyed shutters are a separately audited
family whose set-switch reconstruction can release collision. A generic
`unlocked => no collision` rule would be wrong for at least one representative.

## Exact evidence and remaining unknowns

The authoritative placement rows, source identities, parameter decoders, raw
backing offsets, and conservative import boundaries are in:

- `gz2e01-forest-temple-small-key-door-audit.md`;
- `gz2e01-boss-door-audit.md`;
- `gz2e01-l5-boss-door-audit.md`; and
- `gz2e01-keyed-door-gate-family-audit.md`.

The inventory is complete for those representative families, not for every
actor in the game. Unresolved actor-local geometry, event/cut phases, paired
gate state, non-memory switch domains, overlapping pending key deltas, and
other builds remain explicit obligations. Adding another family requires its
own placement, backing, transient-state, and reconstruction row; resemblance to
one of these families is not evidence.
