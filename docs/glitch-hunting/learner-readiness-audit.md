# Learner-readiness audit

This audit asks whether a learner can perceive, express and prove relevant
state changes. It does not describe how to perform a glitch. It is bound to
Skybook revision `e9104852ff6b87862b67100f58aaa729096b42dc` and the checked
manifest digest `bec0be7e...aedf6`.

The reviewed pages were selected to stress different information boundaries,
not as optimization targets. No setup, input order, route, checkpoint, reward
corridor or successful tape was extracted. “Missing” means that a learner may
be capable of producing the required ordinary controller input but currently
lacks a stable observation or proof channel with which to learn its relevance.

## Current baseline

The action boundary already supports exact per-frame GameCube PAD state,
including simultaneous digital inputs, both analog sticks and analog triggers.
The canonical observation evidence currently retains player motion/action,
camera state, recent controls, player resources/inventory, realized collision
contacts, static geometry, complete active actors, actor identity/kinematics,
base actor state, attention/event participation, complete dynamic collision
shapes, eleven pointer-free Link-rooted actor relationships, Link's cached
background-collision solver modes and work geometry, the global message
session, the bounded priority-ordered event request/participant graph, and the
ordinary attention pass's pointer-free lock/action/check candidate lists with
their rank, type, weight, distance and angle. The candidate lists are also
available to the shared complete-actor-set encoder through explicit masks;
the generic event-manager and pending scene-handoff state now additionally
retains event-data readiness, camera-play state, current event identity/goal,
pending destination, and wipe mode/speed. These channels do not prescribe an
actor, event, destination, route, or input. Other items
in this baseline remain evidence channels unless separately promoted through
learner evaluation.

The audit uses these controlled capability names:

- **PAD**: ordinary digital and analog controller authority;
- **player collision history**: the current solver mode and line/wall
  configuration are captured per tick. A live 30-tick neutral probe
  distinguishes the early empty wall table from the initialized three-circle
  Link solver, and a separate generic 90-tick movement probe observes multiple
  solver modes, wall-table sizes, water modes, ground heights and positions.
  Deliberate contact coverage and a derived bounded floor/wall/ceiling
  transition history remain missing;
- **typed actor state**: actor-specific action, animation phase, timers and
  state-machine values, explicitly masked outside the matching actor type;
- **relationships**: targeting, ownership, attachment, carried/held and
  projectile-to-actor links expressed without pointers. Ordinary attention
  candidate eligibility is captured and joined to actor nodes; retained or
  item-specific target queues remain incomplete;
- **item/projectile state**: typed lifecycle, trajectory, collision and item
  action state;
- **event/loading state**: the message session, pending generic event
  requests/participants, event-manager readiness/current identity/goal, and
  pending destination/wipe state are captured; door, warp and resource queues
  plus their distinct time domains remain incomplete;
- **lifecycle/capacity state**: process creation/deletion, actor slots,
  resource loads and heap/allocation outcomes.

## Representative cases

| Skybook page | Ordinary action authority | Potentially useful state/history | Minimal read-only oracle | Readiness |
| --- | --- | --- | --- | --- |
| `displacement-clipping` | PAD, item/action buttons | player collision history; held/pickup relationship; static and dynamic collision | player crosses a collision-defined topology boundary | Missing observations |
| `floor-clip-ceiling` | PAD | player collision history; simultaneous floor/ceiling clearance and contacts | player enters a region separated by collision topology | Missing observations |
| `bokoblin-wall-clipping` | PAD, targeting, combat | typed enemy state; enemy/player collision relationship; player collision history | player crosses a wall while world identity is unchanged | Missing observations |
| `boomerang-target-storage` | PAD, targeting, item use | target queue; item/projectile lifecycle; retained actor relationships across events | a target relationship persists outside its normal owner/lifetime | Missing observations |
| `door-storage` | PAD, action button | player action; door actor state; event/loading queues and transition history | door/event state persists across an incompatible transition | Missing observations |
| `cutscene-bomb-timer-extending` | PAD, item use, menu/pause | item timer; event state; simulation, event and pause clock domains | timer evolution violates its ordinary clock relationship | Missing observations |
| `arbiters-grounds-death-sword-cycle-delay` | PAD, targeting, combat | typed enemy action/animation phase and timers; bounded history | enemy state-machine cycle or duration violates its ordinary transition | Missing observations |
| `castle-town-observation-deck-actor-slot-exhaustion` | PAD | complete actors; process creation/deletion queue; actor-slot occupancy across loads | actor creation fails while its preconditions remain satisfied | Missing observations |
| `actor-corruption` | PAD | complete actors; typed semantic fields; lifecycle identity; bounded and cross-load history | impossible typed actor state, identity discontinuity, or deterministic crash | Missing observations |
| `goron-mines-dangoro-ebf-pause-manip` | PAD, targeting, combat, pause | typed enemy action/animation frame; pause and simulation clocks | enemy transition occurs at an otherwise invalid typed phase | Missing observations |
| `gorge-skip` | PAD, targeting, item use | player collision history; item/projectile state; actor relationships; local geometry | player reaches a collision-separated region | Missing observations |
| `clawshot-actor-displacement` | PAD, targeting, item use | projectile/attachment relationship; actor collision and kinematics | actor displacement exceeds ordinary collision-constrained motion | Missing observations |
| `arrow-veering` | PAD, aiming, item use | projectile position/velocity; camera and target relationships; bounded history | projectile trajectory changes discontinuously without a collision cause | Missing observations |
| `sticky-rang` | PAD, targeting, item use | boomerang lifecycle, collision and target relationships across events | item remains attached/active outside its ordinary lifecycle relation | Missing observations |
| `universal-map-delay-umd` | PAD, menu/action buttons | event/loading queues; scene identity; simulation and loading clocks across transitions | loading/event commitment is delayed relative to its stable preconditions | Missing observations |
| `ending-blow-wrong-warp` | PAD, targeting, combat | player and enemy typed actions; event and warp queues; cross-event history | committed destination differs from the event’s ordinary destination | Missing observations |
| `epona-seam-clip` | PAD, mount/action buttons | mount relationship; player/mount collision history; static geometry | player or mount crosses a collision-defined seam boundary | Missing observations |
| `hidden-skill-duping` | PAD, combat/action buttons | acquisition flags and resources; typed event state; cross-load history | stable acquisition/resource count violates uniqueness | Missing observations |
| `fishing-rod-duping` | PAD, item/menu actions | inventory/resources; typed item action/animation and lifecycle across transitions | stable inventory/equipment state violates uniqueness | Missing observations |
| `castle-town-squidna-heap-alloc-failure` | PAD | lifecycle/capacity state; resource-load outcomes; heap allocation failures across loads | required allocation or process creation fails deterministically | Missing observations |

All reviewed cases remain attempts for a learner to discover. The table’s
oracles identify only generic invariant violations or outcomes; they do not
reward intermediate positions, actor states, button timings or published
technique steps.

## Consolidated gaps

The representative spread reduces to six observation families worth building,
in this order of cross-case reuse:

1. broad player-collision transition coverage and a derived short-history
   learner view over the now-captured solver state;
2. pointer-free ownership and attachment relationships beyond the current
   Link-rooted target, ride, held/grabbed, retained-item and attention roles;
3. typed item/projectile lifecycle, trajectory, collision and action state;
4. typed enemy/action/animation/timer components with explicit masks;
5. remaining trigger, door, warp and loading queues with explicit clock
   domains (generic message and pending event-request state are now captured);
6. process lifecycle, actor-slot occupancy, resource capacity and allocation
   outcomes.

These are candidate evidence extensions. They must be added through the
read-only automation boundary, remain absent where unsupported, preserve exact
gameplay behavior, and undergo ablation before becoming default learner input.
The audit does not justify exposing pointers, raw process memory, padding or
host-only state.

## Capability backlog

This backlog converts the reviewed cases into reusable subsystem work. It is
not a list of glitches to reproduce. A backlog item may retain state that turns
out to be useful to many behaviors, but it must never contain a published input
order, setup tape, coordinate, timing window, desired intermediate value or
technique-specific reward. The learner, not the collector, is responsible for
discovering which observations predict a successful action sequence.

The canonical episode keeps typed source evidence. Model views, histories and
generic proof oracles are derived later and versioned separately so that a new
representation does not require recollecting gameplay.

| ID | Reusable subsystem signal | Canonical read-only evidence | Permitted derived view or proof | Validation gate |
| --- | --- | --- | --- | --- |
| `collision-history` | Player, mount and actor interaction with static and moving collision | Complete realized contacts, solver modes, surface identity and plane, correction, simultaneous floor/wall/ceiling clearance, moving-background ownership and exact phase | Bounded past-only contact/topology history; generic proof that an entity changed collision-connected region without a world transition | Neutral movement/contact collection must exercise begin, continue, switch and end events; independent capture paths agree; replay reproduces every boundary |
| `relationship-graph` | Ownership, attachment, carry, target, mount, projectile and collision-partner relations | Named pointer-free edges joined to the complete actor population, with explicit absent/unavailable status and lifecycle generation | Masked actor graph; generic proof of a relationship surviving an incompatible owner, lifetime or context | Every present edge joins exactly once, legacy absence remains masked, generic actions produce edge variation, and cold replay preserves the edge sequence |
| `item-projectile-state` | Items and projectiles whose position, lifecycle or action state can interact with other systems | Profile-bound typed creation/action/animation/timer, trajectory, collision, target and owner components | Object-centric history and transition labels; generic trajectory discontinuity or invalid-lifecycle proof | Components are schema-bound to real profiles, absent elsewhere, parity-checked against an independent read path and varied by non-targeted item probes |
| `actor-local-state` | Enemy, NPC, door, mount and other profile-family state machines | Profile-bound action/mode, animation identity/frame, health/status, timers and semantic flags; never member-function pointers or opaque bytes | Masked per-family features and bounded transition history; generic proof of an impossible state, duration or transition | Header/source audit establishes semantics, capture is read-only, profile masks are exact, temporal coverage demonstrates real variation and deterministic replay agrees |
| `event-transition-state` | Dialogue, triggers, doors, warps, scene commitment and resource loading across distinct clock domains | Message and event queues, participants, trigger/door/warp state, pending destination, load/resource phase, and simulation/event/pause/loading clocks | Past-only event graph and clock deltas; generic proof of destination mismatch, persistence across an incompatible transition or violated clock relation | Queue completeness and ordering are validated, actor references join, context boundaries remain explicit, and neutral transition probes vary each supported phase |
| `lifecycle-capacity-state` | Process creation/deletion, actor occupancy, resource and allocation pressure | Complete active actors, semantic pending-create/delete records, slot/resource occupancy and typed success/failure outcomes; no pointers, addresses or guessed capacity | Lifecycle history and generic proof that a required create/load/allocation failed while its observable preconditions held | Counts bind to complete populations, request records are complete and ordered, success/failure varies in generic stress coverage, and ordinary gameplay behavior is unchanged |
| `generic-oracles` | Read-only evidence that an outcome or invariant violation occurred | Collision topology, typed resources, relationship lifetime, event destination, lifecycle result, deterministic crash signature and other already-retained semantic facts | Terminal Boolean proof only; oracle identity is separate from observation and is never an intermediate reward or model input | Each oracle is state-local or explicitly history-bound, has negative controls, cannot reveal future labels at pre-input, and is independently cold-replayable |

Implementation order follows missing reusable evidence, not the order of any
published technique. The current highest-leverage gaps are complete collision
transition coverage, non-Link relationship edges, profile-bound item/actor
state, remaining transition/loading clocks, and semantic lifecycle outcomes.
Existing v18 event-queue, v21 semantic pending-process, and v22 generic
event-transition channels are partial evidence for the last two rows; their
presence does not make those rows complete. V21 records queue order and typed
create/delete process state but not slot/resource capacity, resource-load
results or allocation outcomes.

For every item, promotion requires four separate claims with evidence:

1. **Authenticity:** the value is read from a documented game subsystem at a
   declared boundary and contains no host address, padding or guessed offset.
2. **Completeness:** complete sets are actually complete; optional components
   distinguish absent, unavailable and historically not sampled.
3. **Usefulness opportunity:** generic, non-goal-directed collection shows that
   the value can vary or explains why a constant is still semantically needed.
4. **Determinism:** identical source state and exact PAD reproduce the same
   observation sequence. Any disagreement is a framework bug, not a reason to
   search for a more forgiving tape.

Only after those claims hold may an ablation decide whether a channel enters a
learner view. Skybook page identity and readiness labels remain audit metadata;
they are never runtime features, goals or policy hints.
