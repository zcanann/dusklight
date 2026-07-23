# Ordon q125 reverse curriculum

This report records a real native reverse curriculum over the checked q125
Ordon trajectory. Every frontier keeps the route-proved incumbent as its exact
parent, changes only the permitted residual start frame, and advances only from
a digest-pinned native checkpoint whose exact terminal continuations satisfy
the sealed support policy.

## Policy and authority

- Root action interval: frames `0..126` of the incumbent-relative residual
  surface.
- Initial terminal window: 64 ticks, beginning at frame 62.
- Expansion step: 16 ticks, clamped only at the authenticated root frame 0.
- Expansion threshold: at least 8 successful tapes, 2 behavior classes, and a
  10% exact-success rate.
- Terminal authority: `ordon_spring_load_committed`; the sibling
  `ordon_spring_exit_approach` remains an alternate learning terminal and has no
  promotion authority.
- Execution: four persistent native workers, one deterministic repetition per
  candidate, and a 160-tick exploration horizon.

No coordinate corridor, synthetic state, or gameplay write participates in
the lineage. Each expansion command copies the selected live checkpoint to a
digest-addressed child-owned pin before deriving the child, so later parent
checkpoint pruning cannot invalidate the request.

## Real frontier evidence

| Source generation | Source start | Evaluated tapes | Successful tapes | Behavior classes | Success rate | Child start |
|---:|---:|---:|---:|---:|---:|---:|
| 0 | 62 | 3,904 | 3,444 | 2,725 | 88.2172% | 46 |
| 1 | 46 | 128 | 101 | 55 | 78.9062% | 30 |
| 2 | 30 | 128 | 96 | 50 | 75.0000% | 14 |
| 3 | 14 | 192 | 139 | 74 | 72.3958% | 0 |

The four child request content identities are:

- frame 46: `5b0badbb36a1366dd36487ded13bfd65ff25c335dfe5f1f80257cf91a9aaff18`;
- frame 30: `a7aab224461cead572cc9b2e3d08c356bec2e34fd231e6f64ee66bfcfb795290`;
- frame 14: `4d97ba7b1b3ee9fe9b6e3454c152e1ad0b7ed4edf5f96992c6af40bc8343b459`;
- frame 0: `8ffff5b7e96a455fcd665202a06bff1be9e969fc3efca2853257a48fd26e4fc5`.

Recursive request validation independently reads each pinned request,
execution seal, and checkpoint, recomputes its successful prefix
continuations, and reconstructs the exact permitted child delta.

## Root-frontier result

The frame-0 campaign completed its first 64-candidate generation with 49 exact
terminal successes, 15 failures, and 23 successful behavior classes: a 76.5625%
successful-episode rate. Its best first hit remained tick 125. The evidence is
checkpoint artifact
`build/campaigns/ordon-q125-reverse-curriculum-g4-pinned/checkpoints/checkpoint-00000197.json`,
artifact digest
`b3fe2d93bd15772697e1cd41cdc19d7fef4c9d223c0ad22f876a35f51feae5c1`,
checkpoint content identity
`558d2c146d410441d57b05b5f1125b42d815c5c99297630a3b8eafccda227db8`,
and execution identity
`2d3eb8d4f266d75902dac225b6c7535d3a5d9d30239d14bbb3ec2e95ba58a4a8`.
The crash-safe resume state is
`b10269d942cccaa9720ed284a495c60740555c394761a457ebf35de30e1c3b1d`
after 11,041 charged native ticks.

This closes the curriculum claim: several viable native continuations were
first established near the terminal, every backward step was gated by fresh
exact successes at the new frontier, and the same process reached frame 0 with
diverse successful trajectories.

## Framework performance repairs

The live lineage exposed three framework costs that were fixed rather than
treated as campaign blockers:

- journal appends now reuse the request authority validated at campaign start
  while still refolding the complete durable hash chain and authenticating
  event artifacts;
- retention validation indexes tape-to-evidence membership instead of scanning
  every binding for every tape, and curriculum expansion validates a checkpoint
  once before restoring it;
- historical execution seals no longer rehash runtime files that the child does
  not consume, and alternate-terminal authority is resolved once per validation
  cache instead of recursively validating the full curriculum per evaluation.

On the real pinned requests, unoptimized validation fell from 62.65 seconds to
4.11 seconds for generation 1 and from 74.87 seconds to 4.11 seconds for
generation 2. After the hot-path repairs, generation 3 produced 192 completed
candidates in roughly three minutes, and the root campaign produced its first
64 completed candidates in about one minute.
