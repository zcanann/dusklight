# Collision approach geometry

World facts v14 preserves the geometry that is actually present at an imported
collision/SCLS join without promoting it to a navigation proof.

Each collision-derived encoded-map transition has exactly one
`ExtractedApproachGeometry` record. It binds:

- the transition and its exact `approach_id`;
- source stage, room, collision record, and inventory digest;
- every imported player spawn in that same stage and room; and
- either reconstructed collision geometry or the exact unavailable reason.

For a reconstructed KCL prism, the record contains the three triangle points,
the reconstructed plane normal and offset, and bounds recomputed from those
points. Validation recomputes the bounds, rejects noncanonical floats and zero
plane normals, checks every spawn reference against the same stage/room, and
requires one-to-one coverage of all collision-derived encoded-map candidates.

For a degenerate prism, the record retains `status: unavailable` and the
extractor reason. It does not fabricate a triangle from authored prism fields.

Same-room spawn association is deliberately named `candidate_spawn_ids`. It
does not establish that a spawn and trigger share a connected collision region,
that Link can traverse between them, or that the trigger activation semantics
are source-confirmed. Those remain the geometry obligation and, when present,
the explicit unknown activation requirement on the transition.
