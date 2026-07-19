# Trigger view

Dusklight's runtime trigger view is available under `Shift+F1` -> `Debug` ->
`Trigger View`. Its independent toggles draw scene-transition surfaces and
generic event-area volumes that the running game has actually loaded:

- scene exits are yellow when enabled and orange when disabled;
- event areas are translucent magenta when enabled and dim magenta when disabled; and
- opacity, draw range, and wireframe-only rendering are transient visualization settings.

`Wireframe only` replaces filled surfaces with boundary edges and rings. It is
intended for separating overlapping trigger volumes without accumulated alpha
making their intersection unreadable.

The scene-exit view covers both forms of scene transition used by the
game: collision polygons with a non-sentinel exit ID and realized
`SCENE_EXIT`/`SCENE_EXIT2` actor volumes. Box actors use their exact transformed
extent. `SCENE_EXIT2` is an XZ-only radial test, so the viewer renders its
radius as a vertical extrusion through the configured visible range rather
than inventing a gameplay height bound.

The event/script-area view covers the three generic spatial event families:

- `TAG_EVTAREA` rotated ellipses and boxes, including asymmetric X/Z radii;
- `TAG_EVT` radial script areas; and
- `TAG_EVENT` radial or axis-aligned map-event areas.

These are script activation volumes rather than NPC conversation radii. For
example, the forced Colin conversation blocking the opening `F_SP103` path
tests event-area type 7 until event bit 14 is set; the corresponding magenta
ellipse is the actual spatial boundary that starts that conversation. The
opening `F_SP104` Ilia/Epona scene is driven by a separate `TAG_EVT` radial
area, which is covered by the same toggle.

## Observation boundary

The view is a renderer-side observer. It reads loaded collision backing data,
actor transforms, event-area state, switch/event gates, and SCLS table
availability, then
enqueues debug geometry. It does not call trigger execution methods, perform
collision queries, alter save flags, or write gameplay state. The only native
aperture is a compile-gated friend declaration for the const KCL prism-table
bounds; the adapter body lives in `src/dusk/trigger_view.cpp` and changes no
native class layout. Event-area state uses the same compile-gated, const-only
friend pattern.

Trigger-view output is diagnostic only. It is not part of gameplay traces,
milestone evidence, deterministic state hashes, or TAS scoring.
