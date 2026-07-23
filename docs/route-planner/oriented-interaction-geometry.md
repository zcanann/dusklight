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

## L1 compound interaction

The L1 family uses the player attention position for the main rectangle while
wolf Link is active, then independently checks the player's current-position
local X against a narrower `130` bound. `CompoundInteraction` models those as
form-selected branches:

- human Link tests the player origin against the yaw-oriented `200 x 100`
  rectangle;
- wolf Link tests the observed player attention point against that rectangle
  and independently tests the player origin against a yaw-oriented local-X
  strip bounded by `[-130, 130]`.

`PlayerState::attention_position` is optional and the native observation
projection carries it only when the producer captures the exact
`daPy_py_c::attention_info.position`. A legacy or incomplete observation makes
the active wolf branch unknown. It is never replaced with the player origin,
and the inactive wolf branch does not impose that observation on human Link.
