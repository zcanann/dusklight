# Native return/restart writer trace

Native learning observation v28 records return/restart writer execution between
observation boundaries. The channel is a set of saturating counters outside the
game's save layout, so observing a writer cannot change gameplay state. The
observer consumes and clears the counters after each capture.

The channel keeps thirteen quantities distinct:

- persistent return-place initialization and set calls;
- all `Savmem` executes and the subset whose `NO_TELOP`, event-bit, and room-
  switch guards are eligible;
- restart position/angle/room, start-point, packed room-parameter, and
  last-scene-info setter calls; and
- value changes for persistent return place and each of the four restart
  domains.

This is deliberately execution evidence rather than a raw state diff. A writer
that stores the value already held remains visible, while a value change cannot
exceed the corresponding write count. Eligible `Savmem` execution is likewise
not treated as a return-place change.

## GZ2E01 witnessed trace

One cold, process-booted GZ2E01 run replayed the authenticated Ordon checkpoint
at source frame 506, executed the checked incumbent demonstration for 132
ticks, and reached `ordon_spring_load_committed` at tick 131. The native result
verified source-boundary fingerprint `e7ac8251329f22a5df682bbe5eb2a2ba`
and an eight-tick restored/fresh sequence digest of
`12ffad7f361f096a187f364135c0465c`.

The exact evidence identities are:

| Artifact | Identity |
| --- | --- |
| producer executable SHA-256 | `0b470c4ffd7badef17b49f8122b3c4171580a8bc2aef614732e84e301548d1e5` |
| suffix result file SHA-256 | `14393ee7bacaaacd3f047fdfd1cd491209d5e25c8564f2d804a98bfadef56d43` |
| native v28 shard SHA-256 | `8970478924024c365ea1377b3207c6d23df937438c7e97b7051d4972ea38e201` |
| sealed trace content SHA-256 | `87cb3b12fb7b979aee5520cc24870a510303b29675d7eab43a03fef28a6d71c8` |
| sealed trace file SHA-256 | `443b2f10de3783d44f0b98a27e234125920abe351341631de54c9d43122e3ccc` |

The sealed summary contains 264 observations and 132 event boundaries:

| Evidence | Count | Value changes | Idempotent boundaries |
| --- | ---: | ---: | ---: |
| `Savmem` executes | 264 | n/a | n/a |
| guard-eligible `Savmem` executes | 132 | n/a | 132 without a return-place change |
| persistent return-place sets | 132 | 0 | 132 |
| restart start-point sets | 1 | 1 | 0 |
| restart last-scene-info sets | 1 | 1 | 0 |
| return-place initializes | 0 | 0 | 0 |
| restart place sets | 0 | 0 | 0 |
| restart room-parameter sets | 0 | 0 | 0 |

Every eligible `Savmem` boundary held the same persistent return place before
and after: `F_SP103`, room 1, player status 8. This directly demonstrates why
writer execution and value-change evidence cannot be collapsed.

At post-simulation boundary 638 (simulation tick/tape frame 637), the ordinary
stage request also executed the start-point and last-scene-info writers. The
restart start point changed from 1 to 0, while last speed changed from 0 to 23
and last angle from 0 to -31199. Position, angle, room, and packed room
parameter did not receive writes at that boundary. The persistent return place
again remained unchanged despite one eligible `Savmem` execution.

## Reproduction and validation

The producer writes the v28 shard beside an ordinary suffix-batch result. Seal
and validate its writer trace with:

```text
huntctl learn trace-return-restart-writes \
  --input result.json.episodes.dseps \
  --output return-restart-write-trace.json

huntctl learn validate-return-restart-write-trace \
  --input return-restart-write-trace.json
```

The report binds every source shard, checkpoint, objective, episode, phase,
boundary index, held before/after value, writer counter, summary, and its own
content seal. Validation recomputes the summary from the ordered event stream
and rejects unavailable v28 telemetry, legacy shards, inconsistent counts,
non-finite restart values, reordered events, or tampering.
