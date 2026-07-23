# Boundary state-diff matrix

`StateDiff` is the content-addressed evidence artifact for changes between two
planner snapshots. It retains the exact boundary identity alongside runtime
files, execution context, location/player/world changes, physical-slot deltas,
component payload/binding/provenance deltas, raw byte knownness, and friendly
semantic observations.

The artifact now has the same persistence contract as snapshots: validation,
canonical LF JSON, strict canonical decoding, and SHA-256 identity. Appending
bytes, changing a boundary label, or changing any raw/semantic delta changes or
invalidates the artifact.

The acceptance matrix exercises:

- room and stage transitions;
- runtime save and physical-slot load;
- void reload, savewarp, and title return;
- the stable custom boundary `bit-title-file-zero-entry`; and
- the stable custom boundary `bite-component-splice`.

Every row carries a raw byte/knownness change, a backing-binding change, and a
semantic observation change through canonical encode/decode. BiT and BiTE use
explicit stable custom boundary IDs because they describe compound researched
procedures rather than pretending either is a universal engine boundary. Their
component and runtime-file operations remain fully typed inside the snapshots.

This matrix proves the diff representation across those boundary families. It
does not assert that every build-specific transition has been witnessed; an
unobserved before/after snapshot is still missing evidence rather than an empty
diff.
