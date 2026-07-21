# GZ2E01 post-Zelda cutscene source audit

Status: exact retail topology is extracted. The archive-failure behavior and
return-place writer sequence are still unknown and must not be promoted to an
established route.

## Exact resource chain

The GZ2E01 room archive
`files/res/Stage/R_SP301/R00_00.arc` has SHA-256
`1f8c692843344b7c739d53f940a65eb8f280a857b83879f07db38c94c390fa1d`.
Its `room.dzr` resource has SHA-256
`185cc38ccc5456405380aebccb99f12edf55063de4f048f84b41e9a35f71221e`.

The planner-owned DZR extractor now resolves these records:

- LBNK layer 8 is raw `0702ff`, selecting object archive `Demo07_02`.
- REVT record 0 is STB event `demo07_02`, map-tool ID 4, normal exit 1,
  skip exit 2, raw
  `0202030304ff64010302ffff0064656d6f30375f303200000000ffff`.
- SCLS exit 1 is `F_SP116`, spawn 20, room 0, layer 8 (Castle Town), raw
  `465f5350313136001400f01816`.
- SCLS exit 2 is `R_SP107`, spawn 1, room 3 (Zelda's tower), raw
  `525f5350313037000103f03f00`.

The same room archive's `event_list.dat` has SHA-256
`9b266caac37cb1c582161bb3e04dc2194d944cd1bc2d7040f8f63141ed64b5fe`.
The `extracted-event-list/v1` decoder proves that event `demo07_02` contains:

- PACKAGE `PLAY` with `FileName = demo07_02.stb`, followed by `WAIT`;
- CAMERA `STBWAIT` with `BGCheck = 3`;
- DIRECTOR `MAPTOOL` with `ID = 4`; and
- the parallel ALL/DUMMY staff completion.

`files/res/Object/Demo07_02.arc` has SHA-256
`7a5de4cf1bfb197430a7631b913311e245ed249b16d97616575cb58e001ac11a`.
Its selected `demo07_02.stb` resource has SHA-256
`6417533ffd470dfadcb96ef8a70f2acc7ee9037a4c71f5b864db50b84c176017`.
The JStudio payload itself is not decoded yet.

## Source-backed control flow

`d_s_room.cpp` derives `Demo%02d_%02d` from the current layer's LBNK record,
requests that object archive, waits for its load phase, initializes the event
manager, reloads room actors, and then calls the return-place writer.
`d_event_data.cpp` resolves PACKAGE `FileName`, loads the STB through the current
demo archive, starts JStudio, and resolves DIRECTOR `MAPTOOL` IDs. On event
completion, `d_event_manager.cpp` sends an STB event to REVT's normal exit, or
to its skip exit when the skip flag is active; `d_event.cpp` performs the
ordinary indexed SCLS scene change.

The audited source file SHA-256 values are:

- `d_s_room.cpp`: `15569f9038bb7fb9b956b82b205321c7bebd98ac947c4299739c720b94fe75b4`
- `d_event_manager.cpp`: `bbe434e385c99add82cc4bd0e57923244ce55835535fbd3392e3c086d1ec2c0d`
- `d_event_data.cpp`: `d9cf06093454fc60610bb1b550900feab4e8a0f13d485efcfe2961a8145cd6cc`
- `d_event.cpp`: `663b9f58268a1407827b7de13fc6add512c4eb6b71e1c3dd694df5110dc45eb3`

## Conservative boundary

These facts establish two authored completion destinations. They do **not** yet
establish that actor corruption produces the event skip flag, that a failed
`Demo07_02` load reaches either completion branch, or which return/restart-place
writes execute before such a failure. In particular, the failure must be
modeled as a resource-load predicate with an unknown exceptional suffix, never
as a direct warp to `R_SP107` or `F_SP116`.

The next importer must preserve the incoming return place when an overwriting
writer is proven skipped, invalidate it when writer execution is ambiguous, and
leave the route unknown in established mode until a source or trace witness
identifies the last completed operation. A later savewarp remains a separate
ordinary transition that reads whichever return-place value survived.

## Planner commands

The extraction is independent of Huntctl:

```text
route-planner extract-stage-data \
  --archive files/res/Stage/R_SP301/R00_00.arc \
  --resource room.dzr \
  --output r-sp301-room.json

route-planner extract-event-list \
  --archive files/res/Stage/R_SP301/R00_00.arc \
  --output r-sp301-events.json
```

Both commands reject malformed offsets, overlapping tables, out-of-range graph
references, unsupported data dispatch types, and ambiguous RARC resources.
