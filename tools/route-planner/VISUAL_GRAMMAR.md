# Route Planner visual grammar

This grammar belongs to the route planner. It describes a causal state-and-action
graph, not a TAS timeline, and grants no mechanics or reachability authority to
the browser.

## Authority and identity

- Every rendered node, edge, and region keeps the stable identity supplied by
  the Rust graph projection. Browser positions, zoom, collapse, and active-region
  navigation are presentation only.
- Green acceptance means an authoritative transition application or an authored
  step-to-action relation. Red rejection and amber unknownness come from typed
  Rust assessments. There is no force-connect visual state.
- Selection never changes route semantics. Semantic edits use revision-checked
  route-book edit batches and are reprojected before they appear committed.

## Color and line language

| Meaning | Color | Additional cue |
| --- | --- | --- |
| Current selection | cyan | heavier solid outline |
| Accepted/executable | green | heavier solid edge or node outline |
| Unknown feasibility | amber | dashed edge or node outline |
| Rejected/blocked | red | short dashed edge or node outline |
| Pinned preference | violet | heavier solid node outline |
| Preferred action/method | amber | solid node outline |
| Selected method | green | solid node outline |
| Solver plan | violet | `proof plan` type eyebrow |
| Solver action/state | blue/teal | `proof step` / `proof state` type eyebrow |
| Proven frontier merge | ochre | dashed outline plus typed dominance payload |
| Ordinary causal relation | slate | thin solid edge |

Color is never the only carrier of status: rejected and unknown joins are dashed,
controls expose textual state, and the detail pane retains the typed classification
and diagnostics.

## Canvas geometry

- Nodes use a compact 176 by 52 unit body: type eyebrow above an elided friendly
  label. The stable ID and full contract remain in the detail pane.
- Default layout uses 260-unit semantic columns and 82-unit rows. Goal,
  transition, obstruction, reference-step, and fact families begin in distinct
  columns; saved node coordinates may override this without changing graph
  connectivity.
- Edges use horizontal cubic connectors from the source's right midpoint to the
  target's left midpoint. Stroke widths do not scale with zoom.

## Camera and navigation

- Empty-canvas drag pans. The wheel zooms around the pointer. Explicit plus,
  minus, and Fit controls provide keyboard-accessible equivalents.
- Fit frames only nodes visible in the current region presentation.
- Projected regions are flat-graph encapsulation. Region controls collapse or
  expand them in the full view; double-clicking an owning node enters its region.
  Breadcrumbs follow the projected parent chain and `All regions` returns to the
  complete view. Entering, leaving, or collapsing a region never creates a macro
  action or modifies solver state.
- Palette selection reveals the selected node's projected region before centering
  it, so facts inside a default-collapsed region remain discoverable.
- Solver alternatives may begin collapsed only when graph v10 carries typed
  continuation-equivalence and no-worse-resource evidence. Residual-difference
  alternatives remain expanded; browser containment or color never supplies the
  collapse proof.

## Selection and authoring

- A single node or edge is selected at a time. Clicking empty canvas clears the
  selection and any transient join assessment.
- Transition insertion is the primary direct manipulation: palette transitions
  are draggable to the canvas/current route frontier and use authoritative route
  replay. Earlier steps reject drops because middle insertion is not yet defined.
- Reference steps may nominate a replacement or request removal. Pin, ban,
  prefer, and method-selection controls are visible only for route-book-backed
  editable projects; active choices are both textual and outlined on the canvas.

## Detail panes

- The left section names the selected projection identity and exposes only valid
  commands for its type and project state.
- The middle section summarizes the exact composed mechanics/fact contract and,
  when evaluated, before/after location, execution context, runtime file, player,
  component/fact counts, and typed delta categories.
- The raw pane retains the complete selected projection payload, authoritative
  contract, assessment, closest-state witness, and state inspection/diff. A
  summary must never replace or reinterpret this payload.
- Narrow layouts may hide command and raw panes, but keep the readable inspection
  visible; semantic information remains available through project export and the
  typed service.

## Extension rules

New node families reuse this anatomy unless they require a planner-specific
distinction. New status colors require a textual or geometric cue. Nested proof
views, state nodes, boundary ports, and group-authoring controls must continue to
use Rust-owned identities and must not infer reachability from visual containment.
