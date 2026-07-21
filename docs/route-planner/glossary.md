# Route-planner glossary

These terms are contracts, not interchangeable labels. Serialized schemas use
stable IDs; the UI may show friendlier names without changing their meaning.

## Identity and execution

**Build / content identity**
: One exact immutable game-data identity: platform, region, revision, product
  ID, executable digest, normalized game-data digest, and resource-manifest
  digest. A nearby revision is not the same build. Runtime language and settings
  are deliberately excluded.

**Runtime configuration**
: Mutable selections interpreted with a content identity, such as language and
  settings. An exact planning context is the pair of a content-identity digest
  and runtime-configuration digest.

**Runtime file**
: The currently executing file lifetime and its owned state. It need not be a
  saved slot: BiT file 0 is a slotless, memory-backed runtime file that can hold
  persistent-domain inventory and flags. Ending the lifetime and writing a slot
  are separate operations.

**Physical slot**
: A save destination addressable by the game, such as slots 1–3. A slot contains
  a serialized projection of a runtime file; it is not the runtime file itself.
  Slot 0 is not made real merely because the running file is called file 0.

## State and meaning

**Backing store / backing**
: The physical owner and coordinate of bytes or structured state: for example a
  runtime-file event register, F_SP115 stage-memory byte, session recent-item
  field, live actor instance, or message-flow component. Backing identity
  answers “where is this value?” independently of what it currently means.

**Component**
: The smallest planner-managed state object with one owner, lifetime, binding,
  value/knownness payload, and provenance. Component boundaries are explicit;
  a stage-local bank, process/session field, actor instance, and persistent file
  register are different components even when game memory places them nearby.

**Payload**
: A component's bytes or typed fields, including which bits/fields are known.
  Identical payloads can acquire different semantics after a binding change.

**Binding**
: The current logical association between a component and the entity whose state
  it represents: active runtime file, physical slot, stage, room/zone, actor
  instance, flow session, or explicit custom owner. A transfer glitch changes
  meaning through copy/move/rebind operations; it does not rename the bytes.

**Fact**
: A queryable claim derived from exact backing state, extracted immutable data,
  or another evidenced rule. Friendly facts are views over sources, not a
  second inventory/flag store. Unknown input remains unknown rather than false.

**Writer / reader / gate / latch**
: A writer performs an ordered state operation. A reader observes the current
  backing. A gate decides whether a writer may run. A latch retains a value or
  gate state across later steps. These remain separate so Fanadi locking, held
  return place, and skipped cutscene writes can be explained causally.

## Routes and feasibility

**Transition**
: One state-producing action with exact-context scope, guards, physical
  obligations, ordered effects, unknown requirements, and evidence. Walking
  through a door, talking, saving, loading, BiT, a text interrupt, and a
  cutscene phase are transitions; “did BiT” is not a permanent Boolean fact.

**Candidate transition**
: A transition authorized or encoded by available game data but not necessarily
  physically executable. An SCLS destination stays a candidate until its actor,
  event, side, trigger, and geometry contracts are satisfied.

**Obstruction**
: A directional reason a particular action/approach is blocked in matching
  state, such as a wall, live NPC, twilight form restriction, or locked actor.
  It auto-binds to selected transitions/actions; it is not a global claim that
  two maps are disconnected.

**Obligation**
: A named condition whose proof is needed to activate an action: geometry,
  interaction volume, timing window, form, mount, actor state, player control,
  or another predicate. A technique may discharge an obligation without
  deleting the underlying obstruction.

**Technique**
: A scoped, evidenced state transition or resolver representing an ordinary
  method, glitch, or hypothetical method. Techniques have prerequisites,
  ordered operations, introduced/discharged obligations, and route cost. They
  never grant arbitrary semantic facts by name.

**Refinement / overlay**
: A deterministic layer that adds, replaces, disables, or resolves mechanics
  while retaining provenance and conflict checks. Base extraction, community
  evidence, route-local choices, and “what if?” assumptions can be separate
  layers. Removing a hypothetical layer restores the unchanged base model.

**Route book**
: A versioned authoring artifact that selects goals and expresses required,
  pinned, banned, or preferred actions plus collapsible plan regions. It guides
  search but does not author gameplay effects or override exact mechanics.

**Plan/proof region**
: A collapsible subgraph over real steps and alternatives. Collapse hides
  internal complexity but preserves entry/exit state contracts, proof links,
  costs, and residual-state differences.

**Proof object**
: The causal record for a result: input identities, state snapshots, exact
  actions, guards, obligations/resolvers, evidence, operations, and derived
  facts. “Unreachable under the model” is also a proof claim and must retain its
  failed producer frontier rather than become an authored loss list.

## Evidence terms

**Established / contested / hypothetical / unknown**
: Evidence states, not truthy flags. Established claims are admitted by normal
  search; contested and hypothetical claims require the corresponding research
  policy. Unknown claims are not admitted by either built-in policy and require
  new evidence or an explicit refinement/replacement. Unsupported means the
  planner lacks a valid exact context or schema coverage, which is different
  again from a known-false predicate.
