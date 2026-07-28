# Active tasks: make the route planner usable

This is the unfinished product plan for the interactive route planner. It is
not a changelog, implementation diary, schema inventory, or plan for the route
learning system in `TASKS.md`.

Delete completed work from this file. A service endpoint, serializer, fixture,
or test does not count as a user workflow.

## Product boundary

The planner is an interactive, blueprint-like environment for exploring and
authoring semantic route graphs. It is not a frame-by-frame TAS editor and does
not author learner tactics.

A route author should be able to:

1. create a file-backed workspace;
2. mount exact read-only game/mechanics libraries;
3. ground a scenario in an explicit entry state and runtime configuration;
4. right-click or drag from a typed pin to add a compatible node;
5. connect, branch, replace, and compose route mechanics;
6. inspect relevant state, evidence, and failures on demand;
7. extract and reuse semantic subgraphs;
8. create clearly hypothetical custom nodes;
9. ask the solver for a route or missing producer;
10. save, close, reopen, move, duplicate, fork, export, and delete ordinary
    asset files; and
11. replay the saved graph and obtain the same semantic proof headlessly.

Raw JSON is an interchange/debugging surface, never the primary project or
authoring representation. A workspace is a directory of independently
serialized, versioned assets with stable references.

## Current product truth

The engine has useful typed state, transition, route-book, validation, and
solver machinery. The browser has a graph shell, file-backed workspaces, a
read-only Library, mutable Workspace assets, tabs, basic CRUD, an exact-state
scenario flow, and limited node insertion.

It is still an authoring prototype. A new user cannot yet build, understand,
compose, and replay a meaningful route without implementation knowledge.

## P0 - Establish one coherent interaction model

- [ ] Replace implementation-oriented toolbars and walls of status text with:
  - a compact application bar;
  - a left Content Browser;
  - a central graph canvas;
  - a right Details panel for the current selection; and
  - a collapsible bottom log/problems area that is quiet by default.
- [ ] Make right-click **Add Node** and pin-drag placement the primary authoring
      flows. Both open the same searchable, context-filtered catalogue.
- [ ] Keep nodes compact: title, type, relevant pins, and small status badges.
      Put guards, effects, evidence, state deltas, and diagnostics in Details.
- [ ] Define one command registry used by menus, context menus, shortcuts, and
      the command palette.
- [ ] Preserve graph selection, viewport, open tabs, and pending diagnostics
      across tab switches and non-semantic refreshes.
- [ ] Make every primary action keyboard-accessible and give every semantic
      status a non-color cue.

Acceptance:

- A first-time user can identify where content lives, where graphs are edited,
  and where selected-node details appear without reading documentation.
- The default screen contains no full-state dump or permanent wall of prose.

## P1 - Finish file-backed content and CRUD

- [ ] Keep immutable Library content and mutable Workspace content visibly
      separate. Never present a read-only asset as if it can be edited in place.
- [ ] Use forced root categories for code-authored node kinds, mechanics,
      contexts, fixtures, graphs, scenarios, goals, and custom assets.
- [ ] Support create, rename, move, duplicate, fork, trash, restore, import,
      export, and conflict handling for every mutable asset type.
- [ ] Preserve stable references across rename and move; fail clearly on broken
      or ambiguous references.
- [ ] Add asset-level undo/redo for creation, deletion, move, and rename.
- [ ] Add crash-safe atomic save and recovery without embedding the complete
      workspace in one project document.
- [ ] Provide an explicit “create editable copy/fork” action for immutable
      Library assets.

Acceptance:

- Workspace assets behave like ordinary files.
- Closing and reopening a workspace preserves semantic identities, references,
  tab state, and layout.

## P2 - Ground scenarios and reusable graphs correctly

- [ ] Add a Scenario Root with exactly one explicit anchor:
  - fresh boot plus memory-card configuration;
  - exact card fixture and selected slot;
  - exact recorded snapshot;
  - validated output contract of an upstream graph; or
  - explicit contingent/hypothetical entry contract.
- [ ] Bind every concrete root to exact game, runtime, resource, and state
      identities.
- [ ] Represent a reusable graph by a typed Entry Contract, not an embedded save
      file or fabricated blank card.
- [ ] Preserve unknown values as unknown; omission must never become false,
      zero, empty, or absent.
- [ ] Show grounding as **Grounded**, **Bound**, **Contingent**, or
      **Incomplete**.
- [ ] Permit local reasoning in contingent graphs but prevent them from claiming
      boot-to-goal proof until all entry obligations are grounded.
- [ ] Make scenario anchors and recorded state seeds independently reusable
      files.

Acceptance:

- Graphs can be composed without pretending every graph begins from a blank
  memory card.
- The UI always explains what establishes the displayed state.

## P3 - Complete blueprint-style semantic graph authoring

- [ ] Merge code-authored node kinds, immutable Library mechanics, and mutable
      Workspace subgraphs/custom nodes into one catalogue without conflating
      their authority.
- [ ] Rank catalogue results by connection compatibility, current state,
      context, category, recent use, and text relevance.
- [ ] Define typed execution, state-contract, predicate, and limited data pins.
      Do not expose one pin per field in the complete game state.
- [ ] Prevent invalid connections from committing and explain the exact
      incompatibility before drop.
- [ ] Support insert, remove, replace, reconnect, branches, alternate methods,
      reroute nodes, comments, copy/paste, duplicate, multi-select, alignment,
      and semantic undo/redo.
- [ ] Recompute downstream state after every semantic edit and retain the
      closest valid boundary when later execution becomes blocked.
- [ ] Render executable, blocked, unknown, contingent, and
      context-incompatible edges distinctly.
- [ ] Keep canvas wiring provisional until the Rust authority validates and
      commits the semantic edit.

Acceptance:

- A route can be authored and repaired directly on the canvas without editing
  an ordered list or serialized payload.

## P4 - Add semantic subgraphs and honest custom nodes

- [ ] Serialize subgraphs independently with typed entry predicates, outcome
      predicates, parameters, state projections, costs, and obligations.
- [ ] Add **Extract Selection to Subgraph** and **Collapse to Subgraph Call**.
- [ ] Validate a subgraph at every caller state; one successful invocation must
      not make it universally valid.
- [ ] Support multiple implementations of one outcome as explicit methods and
      show their residual state differences.
- [ ] Preserve selection and viewport when navigating into a subgraph and back
      through breadcrumbs.
- [ ] Compile custom transition nodes through existing validation/refinement
      authority rather than browser-owned behavior.
- [ ] Keep custom nodes visibly hypothetical until evidence and review promote
      them to an established Library mechanic.
- [ ] Reject arbitrary force-state and force-connect nodes.
- [ ] Let custom macro nodes reference subgraphs without copying their internal
      transitions.

Acceptance:

- A user can extract, reuse, inspect, and revise a meaningful route section.
- Hypothetical knowledge cannot masquerade as established game behavior.

## P5 - Make solving and evidence part of the same graph workflow

- [ ] Let the solver propose a complete route, partial producer chain, or
      missing producer as a preview graph that the user can accept, edit, or
      reject.
- [ ] Explain failure in route-author language: missing producer, blocked
      transition, unresolved obligation, unsupported context, or incomplete
      model.
- [ ] Let users pin, ban, prefer, choose costs, and select alternate methods
      from the graph/Details surfaces.
- [ ] Support multiple goals and milestone queries in one scenario.
- [ ] Show only the selected node or edge's relevant Before → Delta → After,
      evidence, and obligations, with user-pinned watches for chosen fields.
- [ ] Export a human-readable route summary and machine-verifiable proof from
      the same saved graph.
- [ ] Require browser, service, CLI, and headless engine replay to agree on
      semantic graph, propagated state, proof, and rejection identities.

## P6 - Deliver one meaningful vertical slice

- [ ] Mount an exact GZ2E01 Library containing only the mechanics and evidence
      needed for the selected slice.
- [ ] From a blank Workspace, create and ground a scenario through ordinary UI
      operations; do not import a preassembled project.
- [ ] Author a coherent route from fresh boot through Ordon progression and a
      meaningful early Forest Temple milestone.
- [ ] Include travel, loading zones, interaction, item acquisition,
      event/cutscene progress, dungeon keys, a locked door, and one obstruction
      or alternate method.
- [ ] Extract and reuse one subgraph from a second compatible state, showing any
      residual-state difference.
- [ ] Add one hypothetical custom node in research mode, inspect its contingent
      result, remove it, and recover the established route.
- [ ] Save, close, reopen, rename, move, duplicate, fork, trash, and restore the
      relevant files without semantic drift.
- [ ] Produce a deterministic route summary and proof matching headless replay.

Do not add full-game, 100%, Any%, multi-platform, or broad model-coverage task
lists until this vertical slice passes. Missing model work should be added only
when it names the exact blocked node in this slice.

## P7 - Usability and release gate

- [ ] Run task-based sessions with people who did not implement the planner.
      Observe workspace creation, scenario grounding, node placement, broken
      connection repair, subgraph extraction, and save/reopen.
- [ ] Require a first-time user to complete a short route without raw JSON,
      source code, or developer coaching.
- [ ] Record failure points and change the product; do not document around an
      unusable interaction.
- [ ] Add screen-reader labels, focus order, scalable text, reduced motion, and
      high-contrast validation.
- [ ] Define and enforce latency budgets for workspace open, catalogue search,
      pan/zoom, semantic edit, downstream replay, and solve preview.
- [ ] Virtualize large content lists and graph surfaces.
- [ ] Run Windows and macOS end-to-end browser jobs rather than skipping them.

The usable-authoring alpha is complete only when the P6 vertical slice passes
through the ordinary UI and independent users can repeat the core workflow.
Until then, call the product a planner engine and authoring prototype.
