# GZ2E01 return/restart writer audit

This audit separates the persistent player return place written by `Savmem`
from transient restart position/angle/room, start point, packed room parameter,
and last-scene information. These are distinct state domains and must not be
treated as aliases.

The canonical machine-readable census is
`tools/route-planner/crates/engine/data/gz2e01-return-restart-audit.json`. It is
bound to exact content `gcn-us-1.0-gz2e01`, extracted-orig bundle SHA-256
`b105cb11f2304b01243fa040cfcc883718c5d34d2021acb47158b21cd8bdb1e2`,
and audit SHA-256
`ec7adca3f055ac97ee2660d88422ca77b0aa89eb5fcd606af8510bf8a6e035ac`.

## SavMem placement and guard census

The exact bundle contains 132 `Savmem` records across 39 stages. Each audit row
retains the archive and resource digests, chunk/layer/index, raw 32-byte record,
parameters, position, angles, set ID, and decoded writer contract.

- 116 room-resource records take their effective save room from the actor's
  room placement, matching `daKytag14_Create`'s `home.roomNo` override. The 16
  stage-resource records use parameter bits 8..15 as a signed room number.
- Parameter bits 0..7 select the save point.
- Angle X selects an event bit that must be set; `0xffff` disables that guard.
  There are 38 placements with this guard.
- Angle Y selects an event bit that must be unset; `0xffff` disables that guard.
  There are 31 placements with this guard.
- The low and high bytes of angle Z select, respectively, a room switch that
  must be set and a room switch that must be unset; `0xff` disables either
  guard. There are 74 switch-set and 12 switch-unset guarded placements.
- Every placement is suppressed while temporary bit `NO_TELOP` is set. Eighteen
  placements have no additional event or switch guard.

Passing these guards makes the actor eligible to execute its setter; it does
not prove that the held return-place bytes changed. A repeated eligible write
of the already-held stage/room/point is a no-op at the value level. Observation
and diff artifacts must therefore retain both writer eligibility/execution and
the before/after return-place value.

## Source writer census

The source census walks repository `src/` C++ and included implementation files,
strips comments, binds every finding to an exact source digest and line, and
finds 32 calls in 24 files:

| State domain | Calls | Files | Meaning |
| --- | ---: | ---: | --- |
| player return-place initialization | 1 | 1 | Clears/initializes the persistent return-place record. |
| player return-place set | 1 | 1 | `Savmem` writes current stage plus decoded room and save point. |
| restart-place set | 19 | 16 | Writes transient restart position, angle, and room. |
| restart start-point set | 1 | 1 | Writes the entrance/start point carried by the restart record. |
| restart-room parameter set | 9 | 7 | Clears or sets the packed restart request parameter, including the PC state-share path. |
| restart last-scene-info set | 1 | 1 | Writes the last speed, mode, and angle tuple during a next-stage request. |

The 19 transient restart-place writes cover player grounding and wolf/demo
paths, boss phases, boss/mini-boss/knob/shutter/spiral doors, and explicit
change/set-restart tags. Their positions and activation guards are local to
those actor procedures; none authorizes a persistent `Savmem` return-place
effect. The start-point and last-scene-info setters run from the generic
next-stage request path and remain distinct from the 19 position/angle/room
writes. The nine parameter writes cover stage request/consumption, title/name/
save-menu transitions, portal-warp setup, and the PC state-share input. A zero
parameter is a distinct write and is not evidence that restart position,
start point, last-scene info, or persistent return place was cleared.

## Reproduction

Starting from the exact extracted-orig bundle:

```sh
route-planner audit-return-restart-writers \
  --repository-root . \
  --bundle GZ2E01.extracted-orig.json \
  --output gz2e01-return-restart-audit.json

route-planner validate-return-restart-audit \
  --input gz2e01-return-restart-audit.json

# Re-census exact source files without re-reading unchanged extracted assets.
route-planner refresh-return-restart-audit-sources \
  --repository-root . \
  --input gz2e01-return-restart-audit.json \
  --output gz2e01-return-restart-audit.json
```

Validation recomputes the content seal, writer counts, strict source ordering,
placement ordering, archive-derived stage/room identity, parameter-derived
target values, and every angle-derived guard. Product IDs and friendly labels
alone never grant this scope.
