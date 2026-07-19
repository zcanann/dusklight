# Trigger view

Dusklight's runtime trigger view is available under `Shift+F1` -> `Debug` ->
`Trigger View`. `Enable Scene Exit view` draws the scene-transition surface
that the running game has actually loaded:

- yellow means the trigger is currently enabled;
- orange means the trigger is currently disabled; and
- opacity and draw range are transient visualization settings.

The first implementation covers both forms of scene transition used by the
game: collision polygons with a non-sentinel exit ID and realized
`SCENE_EXIT`/`SCENE_EXIT2` actor volumes. Box actors use their exact transformed
extent. `SCENE_EXIT2` is an XZ-only radial test, so the viewer renders its
radius as a vertical extrusion through the configured visible range rather
than inventing a gameplay height bound.

## Observation boundary

The view is a renderer-side observer. It reads loaded collision backing data,
actor transforms, switch/event gates, and SCLS table availability, then
enqueues debug geometry. It does not call trigger execution methods, perform
collision queries, alter save flags, or write gameplay state. The only native
aperture is a compile-gated friend declaration for the const KCL prism-table
bounds; the adapter body lives in `src/dusk/trigger_view.cpp` and changes no
native class layout.

Trigger-view output is diagnostic only. It is not part of gameplay traces,
milestone evidence, deterministic state hashes, or TAS scoring.
