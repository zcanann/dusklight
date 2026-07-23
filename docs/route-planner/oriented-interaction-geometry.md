# Oriented interaction geometry

World-axis boxes are not a safe substitute for actor-local interaction tests.
An actor placement can rotate the usable area, and some families test a side
plane independently from their distance rectangle. The planner now represents
that distinction directly.

## Generic contracts

`SpatialVolumeShape::YawOrientedRectangle` stores a world X/Z origin, a signed
16-bit game yaw, and ordered actor-local X/Z bounds. Evaluation applies the
inverse actor yaw before testing the bounds. It intentionally has no Y range:
this shape is for source checks that do not read player height.

`ObligationDetail::Facing` compares an observed signed binary-angle yaw with an
authored target using shortest circular distance. This is not expressible as an
ordinary signed less-than predicate at the `0x7fff`/`0x8000` wrap.

Both contracts fail closed when their observation is absent. An interaction
obligation also continues to require its addressed live actor to be loaded.

## Exact GZ2E01 L5 boss-door import

`daBdoorL5_c::checkArea` rotates the player displacement by the negative door
yaw and accepts `|local_x| <= 200` and `|local_z| <= 100`. `checkFront`
separately requires `local_z > 0`. The facing test compares player yaw against
`door_yaw - 0x7fff` with maximum binary-angle delta `0x4000`.

World facts v10 therefore emit, for each exact imported L5 boss door:

- one placement-origin yaw-oriented `checkArea` rectangle;
- one strict positive-local-Z plane;
- one interaction obligation referencing the rectangle and live actor;
- one plane-side obligation; and
- one circular-facing obligation.

The spatial observation digest binds the exact world-inventory digest, audited
actor-source digest, and placement record ID. Moving or rotating the placement,
changing the source evidence, or importing another inventory changes that
identity.

These records do not claim that the keyhole, event, collision release, scene
change, or restart phases completed. That actor-state obligation remains
explicitly unresolved.

## Deliberate L1 boundary

The L1 family uses the player attention position for the main rectangle while
wolf Link is active, then independently checks the player's current-position
local X against a narrower `130` bound. The current planner snapshot does not
carry that attention point. World facts consequently retain the L1 interaction
as unresolved rather than evaluating both checks against the player origin or
silently excluding wolf routes.
