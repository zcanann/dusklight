# Native tactic observation audit

Source code containing a feature column does not prove that a native campaign
actually supplied the signal to its learner. Run the observation audit against
the route report and every generated training corpus:

```text
huntctl learn audit-tactic-observations \
  --request OPTIMIZATION.json \
  --report ROUTE-REPORT.json \
  --input seed-000/generated-training.dtqc \
  --input seed-001/generated-training.dtqc \
  --output OBSERVATION-AUDIT.json
```

The report is bound to the optimization request, route report, execution plan,
feature and action schemas, root checkpoint, and exact corpus-file digests. It
deduplicates native transitions by replay identity.

For every transition boundary, the audit re-encodes the complete retained
`FactSnapshot` with the route's objective-derived target and compares every
`f32` bit with the feature vector that the learner consumed. It separately
reports native support, feature availability, mismatch count, and observed
variation for:

- player velocity;
- bounded past trajectory;
- camera yaw;
- prompted player-action state;
- complete recent-option kinematics; and
- contact-correlated momentum loss.

The action audit joins each decision's source snapshot to its full typed
applicable-action surface. It reports missing legacy descriptors, roll and A
availability, L-camera-modifier availability, selections, and option-type
coverage. Availability is derived from typed controller factors, not option-ID
parsing.

The policy-signal contract also fails if the campaign used authored route
coordinates, route sequences, multiple waypoint targets, or
benchmark-specific feature names. The single goal coordinate is permitted only
as the objective-derived terminal target; no desired movement, roll, wall, or
Ordon-specific utility is introduced.

The command exits unsuccessfully when evidence is incomplete. In particular,
legacy reports without typed action surfaces remain visibly unauditable rather
than being reconstructed from the current catalog.

