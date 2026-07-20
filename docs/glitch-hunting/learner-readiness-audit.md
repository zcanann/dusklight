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
shapes, eleven pointer-free Link-rooted actor relationships, and Link's cached
background-collision solver modes and work geometry. These are evidence
channels, not prescribed model inputs.

The audit uses these controlled capability names:

- **PAD**: ordinary digital and analog controller authority;
- **player collision history**: the current solver mode and line/wall
  configuration are captured. A live 30-tick neutral probe distinguishes the
  early empty wall table from the initialized three-circle Link solver;
  generic action/contact probes and a derived bounded floor/wall/ceiling
  transition history remain missing;
- **typed actor state**: actor-specific action, animation phase, timers and
  state-machine values, explicitly masked outside the matching actor type;
- **relationships**: targeting, ownership, attachment, carried/held and
  projectile-to-actor links expressed without pointers;
- **item/projectile state**: typed lifecycle, trajectory, collision and item
  action state;
- **event/loading state**: event, trigger, door, warp and loading queues plus
  their time domains;
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
5. event, trigger, door, warp and loading queues with explicit clock domains;
6. process lifecycle, actor-slot occupancy, resource capacity and allocation
   outcomes.

These are candidate evidence extensions. They must be added through the
read-only automation boundary, remain absent where unsupported, preserve exact
gameplay behavior, and undergo ablation before becoming default learner input.
The audit does not justify exposing pointers, raw process memory, padding or
host-only state.
