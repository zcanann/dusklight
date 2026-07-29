# Native tactic campaign resource bounds

Native tactic route report v36 and scratch campaign audit v2 make four
long-campaign resource dimensions explicit.

## Checkpoint memory

The execution plan's `memory_bytes` is an aggregate checkpoint-pool limit.
For a bounded plan, each worker launched in the persistent fleet receives:

```text
min(640 MiB, floor(memory_bytes / active_workers))
```

as the cache capacity in every native request. A budget too small to give every
launched worker a nonzero cache fails before workers launch. Smaller active
views of an already launched throughput fleet retain that original per-worker
capacity because the native cache contract forbids changing bounds in-process.
Their active pool upper bound therefore shrinks with the view. Unbounded plans keep
the historical 640 MiB per-worker ceiling, but cannot pass scratch resource
acceptance because they did not declare an experimental bound.

The campaign audit reports the configured per-worker capacity, configured pool
upper bound, observed peak worker residency, and conservative observed pool
upper bound (`peak worker residency * active workers`). It passes memory only
when observed residency is within the configured pool and that pool is within
the sealed aggregate budget.

## Learner staleness

Two quantities are deliberately separate:

- the lane refresh gap is how many newer fitted snapshots appeared since that
  lane last selected; and
- model replay lag is the durable replay revision minus the newest fitted
  model's replay revision when the lane acquires a selection snapshot.

A large refresh gap means concurrent lanes are producing useful updates; it
does not mean stale evidence was consumed. Model replay lag is the actual
learning bound. The learner fits before pending replay can exceed the sealed
`maximum_stale_replay_revisions`, and selection fails closed if the acquired
model nevertheless exceeds that limit.

Throughput curve v3 judges model replay lag. It retains lane refresh pressure
in the route report for diagnosis without misreporting it as policy
staleness.

## Replay fallback

A missing process-local checkpoint may trigger one authenticated-root
materialization before the primary proposal is retried. The audit reports
fallback count and rate and verifies that fallback replays never exceed
recorded prefix materializations. Each fallback remains visible in restore
accounting rather than becoming an invisible cache miss.

## Checkpoint-owner locality

Every decision retains the available checkpoint owner and proposal worker
slots. The audit recomputes owner-local and misrouted decisions, per-worker
owner assignments, and assignment skew. A process-local restore sent anywhere
other than its recorded owner fails the resource gate. Assignment skew remains
a reported scaling diagnostic because dedicated lane owners intentionally
trade balanced primary work for checkpoint locality.

Scratch discovery acceptance now includes `bounded_campaign_resources`.
Portable evidence bundles recompute the resource audit from their exact route
report and execution plan; merely resealing stale conclusions is rejected.
