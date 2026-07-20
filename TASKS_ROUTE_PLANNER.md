# Twilight Princess route planner: architecture and task plan

## 1. Mission

Build a version-aware route-planning and theorycrafting system for Twilight
Princess that can answer:

- Can a target state be reached from this exact starting state?
- Which known routes reach it?
- Why does a candidate route fail?
- Which requirement, obstruction, or missing technique is responsible?
- What becomes reachable if a proposed exploit or state transfer is assumed?
- Which facts are proven by game data, observed in a run, supplied by community
  knowledge, or merely hypothetical?

The planner must scale in both directions:

- A casual view can say `obtain Fishing Rod` or `beat the game normally`.
- A route author can expand that node into interchangeable subroutes.
- A researcher can inspect individual flags, memory-bank bindings, collision
  obstructions, wrong-state respawns, cross-file operations, and speculative
  bypasses.

This is a causal state-transition model, not a checklist of tricks and not a
hand-authored list of opportunities that each trick loses.

The primary result of a query is one of:

1. `reachable`: the active model contains a complete feasible route.
2. `unreachable_under_model`: the search space is sufficiently closed to prove
   that no route exists under the selected rules and knowledge packs.
3. `unknown`: a route depends on unresolved geometry, incomplete behavior, an
   unaudited state transformation, or an intentionally open research question.

“No known route” must not silently become “impossible.”

---

## 2. Acceptance stories

These stories define the architecture more usefully than a catalogue of data
types. Every implementation phase should preserve them.

### 2.1 Ordinary play

Given a fresh file, the planner can produce a normal route to a chosen milestone,
show inventory and important flags at any step, and collapse routine quest chains
into friendly nodes.

### 2.2 Fishing Rod as a replaceable subgoal

The goal `own Fishing Rod` can be satisfied by at least these plan shapes:

- Talk to the vine NPC, climb, use the hawk, obtain the cradle, return it to Uli,
  and receive the rod.
- Use a chicken to bypass the vine interaction, use the chicken to go out of
  bounds, acquire the cradle carry state, return to Uli, reload as required, and
  receive the rod.
- Mix compatible steps from those routes, such as chicken bypass followed by the
  ordinary hawk step.
- On supported HD rulesets, obtain a rod in another runtime/file context and use
  Auru duplication to transfer the relevant item result to a file that cannot
  reach the ordinary Ordon rod quest.

The collapsed result is simply `obtain Fishing Rod`. Expanding it reveals the
selected proof and its alternatives. The rod is not marked as “lost” by Back in
Time; it becomes unreachable only if all currently enabled producers of the rod
are unreachable.

### 2.3 Back in Time and file 0

Back in Time enters a title-origin, memory-backed runtime file commonly described
as `file 0`. It is not a fourth physical save slot and cannot be saved back under
the file-0 identity, although its serializable state can be written to slots 1–3.

File 0 nevertheless contains ordinary save-domain storage: inventory, equipment,
progression, and miscellaneous flags initialized by the title/BiT path. Those
values persist and propagate like other file state for as long as that runtime
file remains active. Its lack of card-slot backing must not make its contents
temporary flags or hand-authored effects of the BiT technique.

The model must support a route whose current state remains on that slotless
runtime file indefinitely, including a hypothetical future escape that could make
beating the game on file 0 possible. Current known continuations include:

- Save to physical slot 1, 2, or 3 after the required void/title-state handling,
  thereby projecting file-0 state into a persistent slot and ending that file-0
  lifetime.
- Use Back in Time Equipped to carry a pending King Bulblin respawn/equipped
  state into an existing file, again ending the source lifetime while preserving
  only the components that the trick actually transfers.

Saving must therefore be modeled as serialization and runtime-file reattachment,
not as “file 0 becomes slot 0.”

### 2.4 Early Master Sword and layered feasibility

An upper-bound logic pass may conclude that the master sword can be obtained and
that Hyrule Castle or Ganon is logically authorized. A real route may still be
blocked by wolf gates, twilight, collision, void planes, actors, missing movement
capabilities, or an unusable scene transition.

Known and hypothetical options may include:

- The standard Early Master Sword setup with Hero's Clothes and charge attack.
- An Epona out-of-bounds route, with its non-twilight requirement.
- A long rupee-clip path that replaces the charge-attack obligation when paired
  with the required Epona setup.

The solver must explain which obligation each technique discharges and must not
confuse a game-state permission with a physically traversable route.

### 2.5 Nominally local flags transferred to another level

The ordinary model treats the live stage-memory payload as bound to its current
stage and serializes it to that stage's saved bank. With no enabled transfer,
Forest Temple local state behaves as local to Forest Temple.

A theorycraft overlay can introduce a typed transformation that preserves that
raw payload while rebinding it to Temple of Time. The planner then:

- Keeps the bytes unchanged.
- Changes the binding and therefore the friendly interpretation of the bits.
- Re-evaluates all rules that consume Temple of Time's interpretation.
- Retains component-level provenance showing where the payload came from.
- Marks the route hypothetical unless the transformation has evidence.

Deleting the overlay restores the ordinary result. No flag is permanently
declared “untransferable”; instead the base knowledge says that no known transfer
edge exists.

### 2.6 Wrong-state respawn and BiTE

A wrong-flags or wrong-state respawn is a component splice, not a whole-file
teleport. A route may keep inventory and progression from one runtime file, a pending
respawn or equipped state from another, and location/binding from the transition
destination.

Back in Time Equipped should be the first fully modeled example. Other glitches
can reuse the same preservation/clear/rebind operators rather than adding a new
special-case state format.

### 2.7 Fanadi save-location locking

The solver must be able to discover and explain a route with this shape:

1. A save-location actor writes `PlayerReturnPlace`.
2. The player reaches the setup required to interact with Fanadi, including
   Ooccoo where the known glitch requires it.
3. Fanadi enables `NO_TELOP`, which prevents subsequent SavMem writes while the
   gate remains active.
4. The held return-place value survives locations that would normally overwrite
   it.
5. Savewarp reads the held value and enters a normally unexpected map/room/point.

Route order matters. Reaching Fanadi may itself pass writers that replace an
earlier value, commonly leaving a Castle Town-area return point. If a future
bypass reaches Fanadi without those writes, adding that bypass must automatically
make earlier held values available; the Fanadi rule itself should not change.

### 2.8 Theorycraft overlays

A user can enable, disable, add, or propose:

- A new technique that satisfies an existing obstruction.
- A typed state-component transfer.
- A candidate map transition or spawn.
- A changed assumption about collision or an actor.
- A deliberately stronger “assume this obstruction absent” rule.

The base data remains intact. Results identify which overlay assumptions they
depend on and distinguish an evidenced bypass from simply deleting an obstacle.

---

## 3. Core semantic laws

These are non-negotiable design constraints.

### 3.1 Never author derived losses

Do not encode:

```text
BackInTime loses FishingRodOpportunity
```

Encode only causal changes. If Back in Time changes origin, active runtime file,
scene, form, inventory, or flags, ordinary access to the rod quest disappears
because the solver can no longer reach its producers. If Auru duplication is
available on an HD ruleset, the rod becomes reachable again through a different
producer without editing the Back in Time definition.

### 3.2 Tricks are transitions, not flags

A technique has prerequisites, operations, postconditions, evidence, version
scope, cost, and reliability. It is never represented as a permanent Boolean like
`did_bit = true` unless the game itself retains a corresponding state.

### 3.3 Physical storage, logical binding, and semantic meaning differ

“Forest Temple flag 7” is a friendly interpretation, not the underlying storage
identity. Model:

```text
component payload + component kind + current binding + build = interpreted facts
```

The same payload can acquire different meaning after a rebind. “Local” describes
normal reachability and ownership, not an immutable property of the bytes.

### 3.4 State has component-level provenance

After cross-file, wrong-respawn, or duplication tricks, one state may contain
components originating from different files, scenes, or moments. Provenance must
attach to components and writes, not only to the entire snapshot.

### 3.5 Writes and reads are ordered operations

Important values such as return place are not just facts. They have candidate
writers, guards, write suppression, clearing, serialization, and consumers.
A lock preserves the previous value; it does not conjure the desired value.

### 3.6 Game authorization and physical feasibility are separate

Scene-change data and flag checks produce candidate transitions. Geometry,
actors, voids, forms, movement, and other physical conditions determine whether
the transition can actually be activated.

### 3.7 Obstructions are first-class and directional

An obstruction says why a particular approach cannot currently complete. A
bypass resolves that obstruction for a stated approach and scope. It does not
erase the underlying world fact for every route.

### 3.8 Evidence and truth are separate

A fact can be extracted from a build, observed in a trace, reported by a route
pack, or proposed in a what-if overlay. Its truth status and provenance must be
visible and queryable.

### 3.9 Exact builds are the primitive

Rules target concrete platform/region/revision builds. Friendly groups such as
`GCN`, `Wii`, or `HD` expand to exact builds. Version-specific behavior such as
Auru duplication must never leak into unsupported builds.

### 3.10 Collapse preserves proofs; it does not merge states

Two routes that both display as `obtain Fishing Rod` may leave different flags,
positions, carry states, timers, or inventory. They can share a display node only
when the continuation does not care about their differences. Otherwise the UI
must expose the branch or retain separate hidden frontier states.

### 3.11 User assumptions are overlays

Refinements and what-if rules compose on top of immutable base facts. Removing
an obstruction, satisfying it with a technique, and bypassing its approach are
three different operations.

### 3.12 Extracted destinations are inert until activated

A map reference, SCLS destination, or door destination is not itself a traversable
edge. It is a candidate whose activation contract may include actor existence,
interaction side, key or item checks, switches, cutscene state, collision access,
and transition-trigger execution. The solver may traverse it only after every
known hard guard and physical obligation is satisfied; unaudited obligations make
the candidate unknown rather than implicitly usable.

---

## 4. Repository audit and reusable foundations

### 4.1 Existing assets

The repository already contains useful foundations:

- World extraction for placements, spawns, SCLS scene changes, and collision.
- A read-only milestone model.
- A routes crate for concrete timelines.
- A workbench graph with nested single-entry/single-exit subgraphs.
- Existing proof/search infrastructure.
- Save, stage, event, actor, and title-flow source needed to ground mechanics.

These should be reused, but none is presently the complete symbolic causal graph.
Concrete route timelines remain authored route books; extracted data and rules
become the knowledge base; nested workbench graphs become plan-region UI.

### 4.2 Source-grounded save/runtime findings

The current code shows:

- Physical save UI/card slots are 1–3; title-origin “file 0” is a slotless,
  memory-backed runtime file with real save-domain contents, not another card
  slot.
- `dSv_info_c` owns persistent save data plus live current-stage, dungeon, zone,
  temporary, restart, inventory, and related state.
- `dSv_info_c::getSave(stage)` copies a selected per-stage payload into the live
  `mMemory` bank.
- `dSv_info_c::putSave(stage)` copies the live bank back into that stage's saved
  entry.
- The same per-stage `dSv_memory_c`/`dSv_memBit_c` payload contains chest bits,
  switches, item bits, a small-key count, and dungeon-item bits including the boss
  key. Key and boss-key semantics therefore derive from the bound backing store,
  not from the provenance of the pickup that supplied them.
- The SavMem actor writes `PlayerReturnPlace` when its event/switch guards pass.
- SavMem returns before writing while the temporary `NO_TELOP` bit is active.
- Fanadi code sets and later clears the same `NO_TELOP` bit.
- Game start/savewarp consumes `PlayerReturnPlace` to choose the return stage,
  room, and point.

Primary source anchors:

- `include/d/d_save.h`: save object layout and bank accessors.
- `src/d/d_save.cpp`: `getSave`, `putSave`, card serialization, and return-place
  handling.
- `src/d/actor/d_a_kytag14.cpp`: guarded SavMem writer and `NO_TELOP` early return.
- `src/d/actor/d_a_npc_shaman.cpp`: Fanadi's `NO_TELOP` set/clear behavior.
- `include/d/d_save_temp_bit_labels.inc`: `NO_TELOP = 0x1301`.
- `src/d/d_s_play.cpp` and `src/d/d_menu_save.cpp`: title/file/save lifecycle.

This evidence establishes the generic storage and write-gate mechanics. The exact
player-facing exploit sequence, duration, version support, and Ooccoo setup still
need trace or route evidence and should not be inferred solely from these writes.

### 4.3 Audit gaps

Before claiming completeness, inventory:

- Every state bank and its actual lifetime/reset/serialization behavior.
- Stage-to-bank indices and build-specific flag aliases.
- Small-key pickup, delayed award, consumption, and door-unlock writes for every
  relevant door/gate actor family.
- Every writer and reader of return place, restart place, last stage, room, layer,
  and spawn point.
- All SavMem actor placements and their event/switch guards.
- Normal stage load/commit order and failure/void/reload paths.
- BiT/BiTE preservation and clearing behavior per build.
- Known wrong-flags respawns beyond BiTE.
- Auru duplication's exact component transfer semantics in HD.
- Twilight-dependent transition suppression and form/mount restrictions.
- Actor reconstruction rules: which behavior comes from placement parameters,
  persisted switches/items, live actor fields, or room/scene lifecycle.

---

## 5. Architecture

### 5.1 Artifact layers

Keep five artifacts separate:

1. **Build facts** — extracted maps, actors, SCLS, collision, item placements,
   flag labels, and source-audited mechanics.
2. **Knowledge packs** — techniques, obstructions, semantic aliases, route costs,
   and community-verified behavior.
3. **Route books** — curated normal, category, historical, or showcase plans.
4. **Refinement overlays** — versioned community or route-local corrections and
   hypothetical transformations.
5. **Query configuration** — start state, goal, allowed techniques, risk/cost
   policy, evidence threshold, and active overlays.

Route books consume the knowledge base but do not define reachability truth.

### 5.2 Execution environment

A search node is an execution environment, not merely a set of progression flags:

```text
ExecutionEnvironment
  build
  active runtime file
  persistent slot set
  current scene / room / layer / spawn
  player form, mount, pose, and movement state
  state components
  instantiated actors and other live world objects
  pending operations and respawns
  active write gates / latches
  session/title state
  proof and component provenance
```

Runtime-file identity and backing:

```text
RuntimeFile
  runtime_id
  origin: TitleFile0 | NewFile | LoadedSlot(slot_id) | Other
  backing: MemoryOnly | CardBacked(slot_id)
  allowed_serialization_targets
  lifecycle
```

Do not name a memory-backed runtime `slot 0` in the type system. The UI may show
“file 0” as community terminology. Also do not equate `MemoryOnly` with temporary
game state: the runtime may contain persistent-domain inventory and flags and may
remain active indefinitely.

Keep these axes independent:

- **Semantic lifetime:** frame, room, stage, session, runtime file, or serialized
  save.
- **Residency:** live memory, card image, extracted trace, or symbolic value.
- **Backing attachment:** no slot or physical slot 1–3.
- **Serialization policy:** what can be written, to which target, and with what
  normalization.
- **Runtime identity:** which currently executing file the state belongs to.

### 5.3 State components

Represent state as typed components with independent lifetimes:

```text
StateComponent
  component_kind
  payload
  binding
  lifetime
  serialization_owner
  provenance
  interpretation_catalog
```

Candidate component kinds include:

- Inventory and equipment.
- Global persistent event/progression bits.
- Stored per-stage memory entries.
- Live current-stage memory bank.
- Dungeon state.
- Zone and room switches.
- Temporary/event-runtime flags.
- Player return place and restart place.
- Scene, room, layer, spawn, form, and mount.
- Pending respawn, transition, cutscene, or actor state.
- Title/file selection state.

Do not flatten world objects into persistent flags. Keep four related layers:

```text
static placement definition
+ selected room/layer
+ persistent control state
+ transient actor-instance state
= current world behavior and collision
```

For example, a gate's placement and parameters identify its actor behavior; a
stored switch may say it was permanently opened; the live instance may currently
be unloaded, animating, displaced, or collision-disabled. A normal reload
reconstructs the instance from the first three inputs. Actor unload and wrong-state
techniques operate on the live layer unless they explicitly write a backing store.

Candidate lifetimes include:

- Frame/action.
- Room load.
- Stage load.
- Session.
- Runtime-file identity and backing attachment.
- Save serialization.
- Physical slot.

Lifetime and storage are empirical properties with evidence. A component may be
normally cleared on a boundary yet preserved by a technique-specific transform.

### 5.4 Binding and interpretation

The live stage bank has one storage identity and a current binding, for example:

```text
payload: bytes[0x20]
binding: Stage(ForestTemple)
```

Friendly facts are views:

```text
interpret(build, component_kind, binding, offset, mask) -> FriendlyFact
```

After a hypothetical rebind:

```text
payload: unchanged bytes[0x20]
binding: Stage(TempleOfTime)
```

Forest Temple aliases cease to be the active interpretation and Temple of Time
aliases become active. Raw offsets remain inspectable, including unknown bits.

Bindings may be stage-, room-, zone-, dungeon-, runtime-file-, or actor-contextual.
Aliases must be exact-build aware and may be incomplete or contested.

### 5.5 Component transformations and state splices

All ordinary and glitched transitions use a small operation vocabulary:

- `write(component, field, value)`
- `clear(component | field)`
- `initialize(component, policy)`
- `copy(source, destination)`
- `move(source, destination)`
- `preserve(component)` across a boundary that normally changes it
- `serialize(component, owner)`
- `restore(owner, component)`
- `bind(component, context)`
- `rebind(component, new_context)` while retaining payload
- `project(source_runtime_file, destination_runtime_file, component_set)`
- `consume(pending_operation)`
- `set_gate(gate)` / `clear_gate(gate)`

A transition declares its behavior per component. Unmentioned behavior is not
implicitly preserved: it is inherited from a named boundary policy or marked
unknown. This prevents accidental whole-state copying.

Example boundary policies:

- Normal room transition.
- Normal stage transition.
- Void/reload.
- Savewarp.
- Title return.
- Load physical slot.
- Save slotless runtime file to a physical slot.
- BiTE wrong-respawn transfer.
- HD Auru duplication.

This operation language is also the safe boundary for theorycrafting. Users may
propose typed transfers without being given an unconstrained “write arbitrary
memory” primitive unless an explicit research mode requests it.

### 5.6 Save as projection, serialization, and rebind

Saving from file 0 should be described as:

1. Select destination physical slot 1–3.
2. Project the serializable component set from the active, memory-backed runtime
   file.
3. Apply save-time normalization/clearing rules.
4. Serialize into the chosen persistent file.
5. Clear the no-file/title-origin state as observed.
6. Reattach or reload the active runtime as a card-backed file according to the
   actual route.

The source runtime identity and destination card file have different backing and
potentially different identity/lifetime rules.
Any component that does not serialize is lost because of the projection policy,
not because an authored route says it is lost.

Back in Time itself should therefore primarily select/enter the title-origin
runtime file. Facts such as Hero's Clothes, an Ordon weapon, or miscellaneous
progress flags derive from that file's initialized backing stores. The exact
initial inventory and flag image is a build-specific evidence task; it is not an
effect list copied onto every BiT route.

### 5.7 Writer, gate, latch, and reader semantics

For state whose history matters, define mechanisms rather than final facts:

```text
WriterRule
  target
  value expression
  activation location/event
  guards
  priority/order
  evidence

GateRule
  blocked operations
  active predicate
  set/clear mechanisms
  lifetime

ReaderRule
  source
  consuming transition
  interpretation
```

Fanadi save locking then emerges from ordinary rules:

- SavMem is a writer of `PlayerReturnPlace`.
- Its event and switch checks are guards.
- `NO_TELOP` gates that write.
- Fanadi is one setter/clearer of the gate.
- Savewarp/game start reads the last value that successfully wrote.

The solver must retain the concrete held value in state. A Boolean
`save_location_locked` is acceptable only as a derived display fact, never as a
replacement for the gate and value.

This machinery also supports last-writer analysis, actor unload effects, delayed
writes, and hypothetical ways to reach a gate setter without crossing a writer.

### 5.8 Ground facts, derived facts, and friendly names

Ground facts are directly stored or observed:

- Raw bank bits and values.
- Inventory slots and counts.
- Scene/room/layer/spawn.
- Carrier and physical slot identities.
- Pending operations.
- Gate state.

Derived facts are rules over ground facts:

- `has_hero_clothes`
- `can_charge_attack`
- `faron_in_twilight`
- `save_location_write_suppressed`
- `ordinary_ordon_rod_quest_reachable`

The fact catalogue maps raw locations to friendly names, descriptions, build
scope, confidence, and citations. Unknown raw bits remain addressable. Friendly
names never replace raw identity in saved data or proofs.

### 5.9 Transition pipeline

Build traversability in layers:

```text
game authorization
  -> candidate transition
  -> approach-specific feasibility obligations
  -> active obstructions
  -> technique/bypass resolutions
  -> executable transition
  -> witnessed transition (optional stronger evidence)
```

Maintain three graph projections:

- **Upper-bound logic graph:** everything permitted by known internal logic,
  ignoring unresolved physical blockers.
- **Modeled-feasible graph:** candidates whose known obligations are discharged.
- **Witnessed graph:** transitions supported by traces, tests, or sufficiently
  strong route evidence.

A transition can therefore be `feasible`, `obstructed`, or
`feasibility_unknown` without corrupting its logical authorization.

### 5.10 Candidate transitions

Candidate producers include:

- SCLS exits and doors.
- Spawn/restart/savewarp readers.
- Actor scripts and event transitions.
- Item acquisitions and NPC rewards.
- Cutscene and boss completions.
- Form/mount changes.
- Save/load/title operations.
- Technique-defined wrong warps or component transfers.

SCLS data is evidence that a destination is encoded, not proof that Link can
activate it from a given approach.

Each candidate uses an explicit activation contract:

```text
TransitionCandidate
  encoded destination
  activation mechanism / actor
  hard authorization guards
  approach and interaction obligations
  state operations on success
  build and layer scope
  evidence / unknown fields
```

Hard authorization guards are evaluated before the edge becomes executable. A
small-key door that checks a nonzero key count is therefore not traversable merely
because its destination is known. A technique may still avoid the door, reach the
destination trigger another way, or manipulate the guard, but it must name the
specific obligation or alternate activation it supplies.

#### 5.10.1 Dungeon keys as backing-store semantics

Treat keys as ordinary state operations over the bound per-stage store:

```text
small-key pickup
  write live_stage_memory.key_count += 1
  write the pickup/chest persistence bit when applicable

keyed door activation
  require live_stage_memory.key_count > 0
  consume/decrement according to the actor's real behavior
  write the door's persistent unlock state when applicable
  enable or perform the transition

boss-key door activation
  require live_stage_memory.dungeon_items.boss_key
```

The planner does not care which chest or actor originally produced the current
key count. It cares about the value in the store the door actually reads. Normal
stage-bank binding confines keys to their dungeon. A discovered or hypothetical
preserve/rebind/copy bug can expose additional routes automatically, with the
same transfer evidence and provenance rules as any other storage component.

Key count, pickup persistence, door-unlocked state, live door state, and encoded
destination remain separate. Door actor families may implement them differently,
so extraction should preserve actor-specific unknowns rather than inventing one
universal unlock effect.

### 5.11 Obstructions and feasibility obligations

An obstruction definition contains:

```text
id
scope: exact approach / transition / region / build
kind
activation predicate
blocked capability or transition
feasibility obligations
known resolvers
evidence and confidence
```

Kinds may include:

- Collision, wall, gate, ceiling, or ledge.
- Void plane or kill volume.
- Actor/body blocker.
- Twilight barrier.
- Wolf/human/mount restriction.
- Loading-zone reachability or activation direction.
- Camera, animation, or interaction constraint.
- Forced cutscene or state rewrite.
- Unknown geometry/physics.

Obstructions are separate from hard game-state prerequisites such as a door
checking a key flag. The former explain physical realizability; the latter belong
to authorization logic. Some mechanics create both and should expose both layers.

Resolution kinds:

- Satisfy the required ordinary capability.
- Bypass the geometry on that approach.
- Reposition to another side.
- Unload or alter the obstructing actor.
- Avoid the approach through another transition.
- Hypothetically assume the obstruction absent.

### 5.12 Techniques

A technique is a transition provider:

```text
Technique
  id and friendly name
  exact build scope
  input state predicate
  spatial setup / approach
  component operations
  discharged obligations
  introduced obligations or risks
  cost and reliability metadata
  evidence status
```

Techniques should be authored in a validated typed IR. Sources may include Rust,
generated build data, checked data packs, a DSL, or a visual editor, but all must
compile to the same representation.

Examples:

- Back in Time enters a memory-backed, title-origin runtime file whose initialized
  save-domain stores supply inventory, equipment, and progression facts.
- Back in Time Equipped splices selected pending/equipped/respawn state into a
  loaded persistent file.
- Epona OOB discharges named geometry obligations and requires a non-twilight
  mounted setup.
- A rupee clip discharges a particular wall obligation, not “all walls.”
- Auru duplication projects a recently obtained item result across the contexts
  supported by its HD behavior.

### 5.13 Goals and path constraints

Separate end-state goals from path constraints.

Goals:

- Enter Hyrule Castle.
- Defeat Ganon.
- Own Fishing Rod.
- Reach Ordon Spring while Faron remains in twilight.
- Finish on the slotless title-origin runtime file.

Path constraints:

- Never save to a physical slot.
- Preserve Faron twilight.
- Remain on file 0.
- Do not use a named technique family.
- Use only witnessed transitions.
- Permit hypothetical component rebinding.
- Minimize resets, difficulty, or elapsed time.

The same goal can have many constraint-dependent answers.

### 5.14 Solver

The state graph is an AND/OR graph:

- OR: alternative producers for an item, flag, position, or subgoal.
- AND: simultaneous prerequisites for a transition or technique.
- Ordered AND: writer/gate/reader sequences and other history-sensitive setups.

Search operates over concrete-enough environment states, with:

- Exact build and enabled packs in the cache key.
- Component payload/binding/provenance where relevant.
- Dominance checks that preserve continuation-relevant differences.
- Cycle detection across save/load, void, title, and cross-file operations.
- Multi-objective costs such as time, difficulty, resets, evidence quality, and
  hypothesis count.
- Backward relevance analysis from the goal plus forward feasibility from the
  start—the desired “outside-in” behavior.
- Partial proofs when a feasibility obligation is unknown.

The solver must never prune two states solely because their friendly milestone
labels match. It may merge them only after proving equivalence for the remaining
continuation.

### 5.15 Explanations

For each result, retain a proof object capable of answering:

- Which producer satisfied each requirement?
- Which obstruction stopped a candidate?
- Which technique discharged it?
- Which state components were copied, preserved, cleared, or rebound?
- What is the origin of the current value?
- Which last writer established a latched value?
- Which build fact or overlay supplied each edge?
- What minimal missing assumptions would change the result?

Derived lockouts should be presented as cuts in the graph:

```text
Fishing Rod unreachable under this model
  ordinary quest producer: Ordon quest state unreachable
  chicken/cradle producer: required Ordon approach unreachable
  HD Auru producer: disabled because build is GCN
```

That is more useful than `fishing_rod_opportunity = false`.

### 5.16 Route books and fractal plan regions

A route book is a curated preference over the causal graph, not a second mechanics
database. Ship or support books such as:

- Normal completion.
- Standard Any% variants.
- Back in Time research.
- HD cross-file/item-transfer routes.
- Hypothetical/theorycraft showcases.

Plan regions are nested, single-entry/single-exit proof regions when possible:

```text
Obtain Fishing Rod
  selected proof: chicken bypass + hawk + ordinary Uli reward
  alternatives: ordinary cradle quest, glitched cradle carry, HD Auru transfer
```

The view offers:

- Collapse to outcome.
- Expand selected proof.
- Compare alternatives.
- Pin a method.
- Ban a method.
- Expand only unresolved or hypothetical dependencies.

If alternatives have continuation-relevant differences, the collapsed node shows
them as residual effects or keeps separate frontier badges.

### 5.17 Refinement and theorycraft stack

Apply data in a deterministic stack:

1. Extracted/source-audited base facts.
2. Curated built-in knowledge.
3. Enabled community packs.
4. Route-book constraints/preferences.
5. Route-local overrides.
6. Ephemeral what-if overlay.

Every refinement has an ID, exact build scope, author/source, version, confidence,
and replacement/conflict policy.

Supported hypothesis operations:

- Propose a bypass that resolves named obligations.
- Add a candidate technique.
- Add a typed component transfer/rebind.
- Add a candidate writer or suppress a writer under a stated predicate.
- Assume an obstruction absent in a precise scope.
- Supersede a disputed fact while retaining both records.

The result UI lists all active assumptions. A hypothetical edge must not become a
silent fact after export or sharing.

### 5.18 UX surfaces

#### Route canvas

- Pan/zoom graph with collapsible plan regions.
- Distinct styling for logical, obstructed, feasible, witnessed, and hypothetical
  edges.
- Alternative-producer branches and selected proof.
- Route costs and version badges.

#### State inspector

At every selected node show:

- Inventory/equipment.
- Friendly global and local flags.
- Raw component bytes/bit positions.
- Current bindings and lifetimes.
- Scene/room/layer/spawn/form/mount.
- Runtime-file identity/backing and physical slots.
- Pending operations.
- Return/restart values, last writers, and active gates.
- Bound dungeon key count/items, key provenance, and consumed/unlocked door state.
- Static placement versus persisted control versus live actor-instance state.
- Component provenance and active hypotheses.
- Diff from the previous node.

#### Requirement/obstruction inspector

- Friendly explanation.
- Raw predicate.
- Producers or resolvers.
- Why each alternative is currently unavailable.
- Evidence and build scope.

#### Theorycraft editor

- Clone a route/query into a sandbox.
- Toggle packs and individual assumptions.
- Draw a proposed edge or choose a typed component transform.
- Preview a component under a different binding before enabling it.
- Compare reachability and proofs before/after.
- Export a small reviewable refinement pack.

#### Authoring workflow

1. Pick start and target states.
2. Ask the solver for candidate plans.
3. Pin, ban, or prefer methods.
4. Collapse uninteresting regions.
5. Expand unusual proofs and inspect state diffs.
6. Add a route annotation or create a refinement if mechanics data is missing.
7. Validate build scope, evidence, and regressions.

### 5.19 Automation and human refinement boundary

The system should extract or source-derive the factual skeleton wherever
practical:

- Maps, rooms, layers, placements, encoded destinations, and spawns.
- Backing-store layouts and ordinary reads/writes.
- Actor parameters and recognizable hard guards such as key, boss-key, item,
  switch, event, form, or twilight checks.
- Pickup effects, key counts, and persistent unlock writes where code/data makes
  them decidable.
- Ordinary actor reconstruction and transition activation mechanisms.
- Collision and approach facts that can be established mechanically.

Human refinements supply knowledge that extraction cannot prove reliably:

- Geometric reachability and approach-specific blockers.
- Actor unloads, clips, alternate loading-zone activation, and other techniques.
- Timing, reliability, difficulty, and practical route cost.
- Poorly understood flags, actor parameters, and lifecycle behavior.
- Cross-version observations, wrong-state preservation matrices, and hypotheses.

Coverage is tracked per fact and obligation, not as one percentage for the whole
game. Extraction may cover most topology while a small number of unknown physical
obligations still dominate route validity. Human refinements are versioned,
evidenced overlays over the generated base rather than silent edits to it.

---

## 6. Dependency-ordered implementation plan

### Phase 0 — Evidence inventory and terminology

- [ ] Catalogue exact supported builds and stable build IDs.
- [ ] Inventory extracted world-data schemas and their missing fields.
- [ ] Catalogue all save/runtime components and reset boundaries.
- [ ] Audit title, no-file, save-slot, load, void, death, and savewarp flows.
- [ ] Audit SCLS and actor-driven transition consumers.
- [ ] Audit keyed door/gate actor families, their key/boss-key guards, consumption,
      persistent unlock writes, and alternate activation paths.
- [ ] Inventory static placement, persistent control, and transient instance state
      for representative actor families.
- [ ] Audit SavMem placements, guards, and all return/restart-place writers.
- [ ] Record known BiT, BiTE, Auru duplication, wrong-flags respawn, Fanadi lock,
      and Ordon/twilight route evidence without prematurely encoding conclusions.
- [ ] Establish a glossary: build, runtime file, backing, slot, component, payload,
      binding, fact, technique, obstruction, obligation, refinement, route book,
      proof.

Deliverable: an evidence index and a list of explicit unknowns.

### Phase 1 — Typed semantic IR

- [ ] Define exact build selectors and groups.
- [ ] Define execution environment, runtime-file identity, backing attachment,
      and serialization policy independently.
- [ ] Define typed components, bindings, lifetimes, and serialization owners.
- [ ] Define raw/friendly fact catalogue and derived-rule IR.
- [ ] Define component operation and boundary-policy IR.
- [ ] Define transition, writer, gate, reader, technique, obstruction, and resolver
      schemas.
- [ ] Require every candidate transition to carry an activation contract with
      hard guards, physical obligations, effects, and explicit unknown fields.
- [ ] Define static world-object, persisted control, and live actor-instance
      representations plus reconstruction rules.
- [ ] Define goals, path constraints, costs, and evidence status.
- [ ] Define refinement pack manifests, conflicts, and deterministic precedence.
- [ ] Add strict schema validation and stable IDs.

Deliverable: one validated runtime representation independent of authoring format.

### Phase 2 — Observation, snapshots, and diffs

- [ ] Extend the current tape/trace format with runtime-file identity and backing
      attachment.
- [ ] Capture physical slots separately from the active runtime.
- [ ] Snapshot typed components plus unknown/raw regions where possible.
- [ ] Record binding changes and component provenance.
- [ ] Record return/restart values, gates, and relevant actor writes.
- [ ] Produce semantic and raw diffs across room load, stage load, save, load,
      void, title, BiT, and BiTE boundaries.
- [ ] Make unsupported/unobserved fields explicit rather than defaulting false.

Deliverable: replayable state evidence that can validate transition rules.

### Phase 3 — Base mechanisms and upper-bound graph

- [ ] Import maps, rooms, layers, spawns, SCLS, placements, and actor transitions.
- [ ] Model ordinary item/NPC/event producers.
- [ ] Implement normal bank commit/load and binding changes.
- [ ] Derive bound small-key counts and dungeon items from per-stage memory.
- [ ] Import hard door/actor guards and their state operations where decidable.
- [ ] Reconstruct live actor behavior from placement, layer, persisted state, and
      instance lifecycle.
- [ ] Implement save/load/title/runtime-file operations.
- [ ] Implement writer/gate/reader evaluation and last-writer provenance.
- [ ] Generate the upper-bound authorization graph.
- [ ] Keep extracted destinations non-executable until their activation contracts
      are discharged.
- [ ] Mark candidates whose activation physics remain unknown.

Deliverable: the intentionally permissive logic graph with honest uncertainty.

### Phase 4 — Component transfers and state splices

- [ ] Implement per-component transition policies.
- [ ] Implement project/preserve/clear/copy/rebind operations.
- [ ] Prevent accidental preservation of unspecified components.
- [ ] Support mixed provenance after cross-file operations.
- [ ] Encode an evidence-backed BiTE preservation matrix.
- [ ] Encode HD Auru duplication's actual transferable result and constraints.
- [ ] Add a hypothetical local-bank rebind refinement for testing.
- [ ] Add diagnostics for aliases that change under a binding.

Deliverable: one generic system for known and proposed wrong-state transfers.

### Phase 5 — Physical feasibility and obstructions

- [ ] Derive approach geometry from collision and spawn data where possible.
- [ ] Define obligations for reaching/activating each candidate transition.
- [ ] Import authored obstructions without mutating build facts.
- [ ] Support direction, form, mount, twilight, actor, void, and layer scope.
- [ ] Classify candidates as feasible, obstructed, or unknown.
- [ ] Expose upper-bound versus modeled-feasible graph diffs.

Deliverable: flag-permitted nonsense is visible but no longer reported as a
verified route.

### Phase 6 — Technique and refinement packs

- [ ] Build validated authoring for techniques and obstruction resolvers.
- [ ] Encode exact setup, component operations, discharged obligations, and cost.
- [ ] Add built-in packs for ordinary movement and selected sequence breaks.
- [ ] Add route-local and ephemeral what-if overlays.
- [ ] Distinguish bypass, avoid, supersede, and assume-absent operations.
- [ ] Add import/export and conflict diagnostics.

Deliverable: researchers can extend the model without editing core code.

### Phase 7 — Solver

- [ ] Implement backward relevance expansion from goals.
- [ ] Combine it with forward stateful feasibility from the start.
- [ ] Support OR producers, AND requirements, and ordered writer/gate/read setups.
- [ ] Implement state hashing, dominance, cycles, and continuation-safe merging.
- [ ] Support exact build, evidence, technique, runtime-file, and path constraints.
- [ ] Add multi-objective cost and K-alternative plan search.
- [ ] Return reachable, unreachable-under-model, or unknown.
- [ ] Report minimal missing obligations/assumptions where practical.

Deliverable: a headless query API and deterministic fixture suite.

### Phase 8 — Proofs and explanations

- [ ] Retain causal proof objects for every result.
- [ ] Explain derived lockouts as failed producer cuts.
- [ ] Explain obstructions and the resolver chosen for each approach.
- [ ] Show component transformation and provenance histories.
- [ ] Show last-writer and gate history for latched values.
- [ ] Label all hypothetical and low-confidence dependencies.
- [ ] Generate concise collapsed summaries and fully expanded research views.

Deliverable: every route and failure is inspectable rather than magical.

### Phase 9 — Workbench UX and authoring

- [ ] Adapt nested workbench subgraphs into plan regions.
- [ ] Add route canvas, alternatives, pin/ban/prefer, and collapse controls.
- [ ] Add inventory/flag/component state inspector with before/after diff.
- [ ] Add raw flag catalogue search and friendly aliases.
- [ ] Add obstruction and requirement inspectors.
- [ ] Add theorycraft component-transfer and bypass editor.
- [ ] Show active packs, overlays, exact build, confidence, and route costs.
- [ ] Keep route annotation separate from mechanics refinement.

Deliverable: one UI suitable for both simple routes and deep research.

### Phase 10 — Evidence and proof integration

- [ ] Match planned edges to trace/tape observations.
- [ ] Validate postconditions and component preservation against snapshots.
- [ ] Promote witnessed edges without erasing lower-confidence alternatives.
- [ ] Attach source, extraction, trace, video, or community citations.
- [ ] Add tools to identify facts used by many routes but supported weakly.
- [ ] Report extraction coverage separately for topology, hard guards, backing
      stores, actor lifecycle, and physical feasibility.

Deliverable: route confidence is mechanically explainable.

### Phase 11 — Vertical slices

#### 11A. Fishing Rod

- [ ] Model ordinary vine/hawk/cradle/Uli producers.
- [ ] Model chicken vine bypass, OOB, cradle carry state, reload, and Uli reward.
- [ ] Permit compatible mixing where real predicates allow it.
- [ ] Model HD Auru duplication as an alternate producer.
- [ ] Prove GCN BiT does not author a rod loss; it merely lacks reachable producers.
- [ ] Collapse all methods into `obtain Fishing Rod` with residual-state safety.

#### 11B. Back in Time/file 0

- [ ] Enter the slotless title-origin runtime file and import its exact initialized
      persistent-domain stores per build.
- [ ] Show physical slots 1–3 separately.
- [ ] Model void/title-state handling and save projection to a chosen slot.
- [ ] Model BiTE as a selected component splice into an existing file.
- [ ] Allow an unsaved file-0 goal and hypothetical escape overlay.
- [ ] Explain exactly which components die when a file-0 lifetime ends.

#### 11C. EMS to Hyrule Castle/Ganon

- [ ] Produce upper-bound logic path.
- [ ] Introduce geometry/twilight/form/mount obstructions.
- [ ] Encode standard EMS setup.
- [ ] Encode Epona OOB non-twilight constraint.
- [ ] Encode rupee clip as a scoped replacement for the charge-attack approach.
- [ ] Show how route results refine as obstruction knowledge is enabled.

#### 11D. Local-bank rebind

- [ ] Snapshot a Forest Temple-bound payload.
- [ ] Add hypothetical preservation and Temple of Time rebind.
- [ ] Verify raw bytes remain identical while aliases change.
- [ ] Derive downstream effects only from the new interpretation.
- [ ] Display mixed provenance and hypothesis dependency.
- [ ] Remove overlay and verify base reachability returns unchanged.

#### 11E. Fanadi save-location lock

- [ ] Model SavMem writer, event/switch guards, and placements.
- [ ] Model `NO_TELOP` as a write gate with observed lifetime.
- [ ] Model Fanadi setter/clearer and Ooccoo/setup prerequisites.
- [ ] Retain the exact last successful `PlayerReturnPlace` write.
- [ ] Model savewarp as a reader of the held value.
- [ ] Search for setup orderings and explain intervening writes.
- [ ] Add a hypothetical Fanadi-access bypass and verify earlier return locations
      become usable without modifying the core lock mechanism.

#### 11F. Faron-twilight return research

- [ ] Define goals for Goats map, Ordon Village, outside Link's house, Link's
      house, and Ordon Spring while Faron remains in twilight.
- [ ] Enumerate SCLS, spawn, savewarp, void, death, title, cutscene, actor, and
      technique-provided incoming transitions to each target.
- [ ] Apply twilight, form, collision, and activation obstructions per approach.
- [ ] Include BiT/BiTE, held return place, wrong-state respawns, OOB, and proposed
      component-transfer hypotheses where scoped plausibly.
- [ ] Report reachable, blocked, and unknown candidates with exact missing
      obligations instead of flattening them into “no.”

#### 11G. Lanayru spirit and Vessel of Light

- [ ] Locate the spirit actor/event flow and every build-specific placement.
- [ ] Identify the raw event bits, temporary bits, room/layer, form, twilight,
      approach, and cutscene prerequisites for the spirit to appear.
- [ ] Separate appearance, interaction, cutscene start, Vessel grant, and post-grant
      state into distinct transitions rather than one milestone.
- [ ] Identify all writers and consumers of the Vessel and tear-count state.
- [ ] Test whether alternate entrances, wrong layers, wrong-state respawns, or
      component transfers can satisfy or bypass individual prerequisites.
- [ ] Produce both a friendly explanation and the exact raw predicate for each
      supported build, with unknown conditions called out explicitly.

#### 11H. Keyed dungeon transition

- [ ] Extract one representative small-key door's encoded destination without
      making it immediately executable.
- [ ] Derive the bound dungeon key count from the live per-stage backing store.
- [ ] Model key pickup provenance independently from the fungible count.
- [ ] Audit and model the door actor's guard, key consumption, persistent unlock
      write, live animation/collision, and reload reconstruction.
- [ ] Verify any key from the same bound dungeon store can satisfy the door.
- [ ] Add a hypothetical key-store preserve/rebind overlay and verify it opens
      routes only through backing-store semantics, with hypothesis provenance.
- [ ] Verify an OOB route that avoids the door does not falsely mark the door
      unlocked or consume a key.

---

## 7. Regression and acceptance matrix

Every fixture specifies exact build, start environment, active packs, query,
expected classification, and key proof facts.

| Fixture | Required assertion |
| --- | --- |
| Normal fresh file | Ordinary milestones and inventory progress correctly. |
| BiT file 0 | Runtime is memory-backed with persistent-domain contents; no physical slot 0 exists. |
| File-0 save | State projects only to slot 1–3 and applies save policy. |
| Unescaped file 0 | Lack of slot backing is not equated with dead; hypothetical continuation remains representable. |
| GCN rod after BiT | All enabled rod producers fail causally; no authored loss marker exists. |
| HD rod after BiT | Auru producer can restore reachability only on supported builds. |
| Rod route collapse | Alternative proofs collapse only when residual differences are safe. |
| EMS upper bound | Logical authorization appears before geometry is supplied. |
| EMS obstruction | Physical blocker removes route only from feasible projection. |
| Epona OOB | Route is rejected while twilight/mount predicate fails. |
| Local bank normal flow | Stored stage entries load/commit to the proper binding. |
| Hypothetical rebind | Payload remains stable, aliases and consequences change. |
| BiTE splice | Only declared components cross; provenance remains mixed. |
| Fanadi gate off | Passing SavMem updates return place. |
| Fanadi gate on | Passing SavMem preserves the prior value. |
| Fanadi ordering | Intervening pre-lock writer determines held value. |
| Hypothetical Fanadi access | New access edge exposes prior values without editing gate rules. |
| Lanayru spirit appearance | Appearance, interaction, and Vessel grant have separately verified predicates. |
| Encoded keyed door | Destination alone is not executable while its hard guard is unsatisfied. |
| Fungible small keys | Different pickups feed the same bound count and can satisfy the same door. |
| Key consumption/unlock | Count, persisted unlock, and live actor state update independently. |
| Key-store rebind hypothesis | Target door derives access from transferred backing state and reports the overlay. |
| Door-avoidance OOB | Alternate route neither consumes a key nor mutates the door's persisted state. |
| Overlay isolation | Disabling an overlay restores identical base results. |
| Unknown geometry | Query returns unknown, not impossible or reachable. |
| Exact build scope | Unsupported techniques never appear through group leakage. |

Also require:

- Golden state diffs for room, stage, void, save, load, title, and splice
  boundaries.
- Schema round trips and stable IDs.
- Solver determinism for identical packs and cost policies.
- Property tests that no undeclared component survives a boundary.
- Property tests that collapsed plan regions preserve all valid continuations.
- Conflict tests for competing aliases, refinements, and obstruction claims.

---

## 8. Open research questions

These should remain explicit unknowns until evidence closes them:

- Exact file-0 component initialization and every build difference.
- Exact BiTE preservation/clearing matrix and whether multiple setups share it.
- Other known respawn-with-wrong-flags or wrong-bank glitches.
- Whether any existing technique can preserve/rebind live stage memory across a
  context that normally replaces it.
- Complete semantic mapping for stage, zone, dungeon, and temporary flag banks.
- Complete small-key consumption and persistent-unlock behavior across all
  door/gate actor families and exceptional stages.
- Exact Fanadi/Ooccoo glitch sequence, `NO_TELOP` lifetime, clear conditions, and
  build support.
- All return-place writers encountered on paths to Fanadi.
- Whether actor unload/order can extend or alter the write gate.
- Exact Auru duplication payload, source/destination restrictions, and reset
  behavior.
- Which Faron-twilight target maps have encoded but physically inaccessible
  incoming transitions.
- How much geometry feasibility can be derived automatically versus authored.
- Which actor-instance fields can be observed generically and which require
  actor-specific adapters or trace annotations.
- Unresolved build differences or hidden actor/event conditions in Lanayru's
  appearance and Vessel-of-Light sequence after the 11G audit.
- Evidence thresholds for claiming `unreachable_under_model` in partially audited
  regions.

---

## 9. Definition of done

The first serious release is complete when:

- Reachability is derived from transitions and state, never authored loss lists.
- Slotless file 0, its persistent-domain contents, physical slots, save
  projection/reattachment, and cross-file provenance are represented accurately.
- Raw storage, bindings, friendly flag interpretations, and component lifetimes
  are independently inspectable.
- Wrong-state respawns and hypothetical local-flag transfers use generic typed
  component operations.
- Fanadi save locking emerges from writers, a `NO_TELOP` gate, the held value, and
  a savewarp reader.
- Authorization, physical obstruction, known bypass, and witnessed execution are
  distinct layers.
- Encoded destinations remain inert until hard guards and physical activation
  obligations are satisfied; dungeon keys derive from their bound backing store.
- Static placements, persisted actor-control state, and transient actor instances
  are modeled separately across load/unload boundaries.
- Users can author versioned refinement packs and ephemeral what-if scenarios
  without mutating base data.
- The solver supports alternative producers, ordered setups, exact builds,
  path constraints, costs, and honest unknowns.
- The UI scales from a collapsed milestone route to raw flags, component history,
  and full proof graphs.
- Fishing Rod, BiT/file 0, EMS/obstructions, local-bank rebind, Fanadi locking, and
  Faron-twilight return research all pass their vertical-slice fixtures.
- Lanayru spirit appearance, interaction, and Vessel grant are modeled as
  separately testable transitions with raw and friendly requirements.
- Every displayed route can explain why it works, every rejected route can explain
  what blocks it, and every hypothetical route names its assumptions.
