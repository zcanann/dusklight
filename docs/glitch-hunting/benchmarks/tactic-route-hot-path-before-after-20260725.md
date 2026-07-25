# Tactic-route hot-path before/after checkpoint

Date: 2026-07-25

This is an intentionally incomplete P0 benchmark checkpoint. It records the
measurements that can be compared honestly from the existing sealed Ordon
campaign and identifies the measurements that still require a controlled
legacy rerun. It does not satisfy or close the benchmark task in `TASKS.md`.

## Sealed workload

Both runs used:

- optimization request
  `routes/Glitch Exhibition/ordon-exit-approach-learning/benchmarks/ordon-exit-approach-tactic-q-discovery-v1.request.json`
  (`296778d879e533f1b50f5eb7e5fbf886e2d0c23f230716c9c91b213973ef45cc`);
- execution binding
  `build/campaigns/ordon-exit-approach-second-pair-20260725/execution-v4/execution.json`
  (`47067b4d7a29dfbf9821fa9f2663891023ecc05e30e40cca552fa02f3f77ef9b`);
- seed `181081`;
- 64 decisions per seed;
- refit every decision; and
- epsilon `350000` per million.

The historical artifact is
`build/campaigns/ordon-exit-approach-second-pair-20260725/tactic-route-v5`.
It was produced by the JSON checkpoint/materialized-projection implementation
immediately before commit `3256fc212d`. The current artifact is
`build/campaigns/ordon-exit-approach-second-pair-20260725/tactic-route-p0-current-20260725`,
produced at commit `64805f334b`.

The current command was:

```text
tools/huntctl/target/debug/huntctl.exe learn tactic-route
  --repository-root C:\Projects\dusklight
  --request "routes/Glitch Exhibition/ordon-exit-approach-learning/benchmarks/ordon-exit-approach-tactic-q-discovery-v1.request.json"
  --execution build/campaigns/ordon-exit-approach-second-pair-20260725/execution-v4/execution.json
  --output build/campaigns/ordon-exit-approach-second-pair-20260725/tactic-route-p0-current-20260725
  --seed 181081
  --decisions-per-seed 64
  --branch-every 8
  --refit-every 1
  --epsilon-per-million 350000
```

## Semantic equivalence

Both reports contain four decisions, three useful decisions, one successful
seed, and 89 native ticks. Each decision has the same selected tactic, route
suffix length, and resulting snapshot digest:

| Decision | Tactic | Accumulated ticks | Result snapshot |
| ---: | --- | ---: | --- |
| 0 | `goal.seek.route.03` | 40 | `4018a2f19b014b7d2e4700b4f401729f0bb8e87ad3730179491d2d6ae16a3db2` |
| 1 | `goal.seek.route.03` | 80 | `2e9641085c25691de783cef5f8cdcae6bc3256ab00f8f4f0eaebe33c6bd79940` |
| 2 | `attack.jump` | 86 | `04b53a8dbae60c29b4b61bd66e76f462f688cbb4011194a6b593c8858a01ff20` |
| 3 | `attack.jump` | 89 | `75faea5689b8bd2f8a59c5f6ece275758eb8a3ece72a5595d8bb36da40f8aabf` |

## Comparable measurements

| Measurement | Historical | Current | Change |
| --- | ---: | ---: | ---: |
| Files written | 274 | 342 | +68 |
| Bytes written | 13,772,714 | 10,540,182 | -3,232,532 (-23.5%) |
| Reported campaign wall time | 41.987991 s | 46.915898 s | +11.7% |
| Evidence projection and persistence | 0.113312 s | 1.761077 s | 15.5x |
| Useful decisions/s | 0.071449 | 0.063944 | -10.5% |
| Native ticks/s | 2.119653 | 1.897011 | -10.5% |

The current artifact's large families are 6,312,466 bytes of retained JSON
proof/reporting objects, 2,383,179 bytes of native episode shards, 952,999
bytes of native requests, 580,089 bytes of native results, and only 5,539
bytes across the operational binary checkpoint and journals. Thus the
operational format is compact, while native scratch and the retained proof
projection still dominate bytes and file count.

The current process-group peak working set was 803,676,160 bytes (766.4 MiB),
sampled every 100 ms across the `huntctl` and native `dusklight` processes.
External elapsed time was 95.548 s. The report accounts for only 46.916 s, so
the existing report boundary omits substantial process startup, initial
restore, finalization, or shutdown time. No end-to-end throughput claim should
use the report wall clock until that boundary is fixed.

## Missing measurements and conclusion

The historical run did not record peak memory or an external end-to-end wall
clock. Its `evidence_projection_and_persistence_micros` counter combines
serialization, projection, and filesystem work, so it is not a
serialization-only measurement. Those missing values cannot be reconstructed
from the retained artifacts.

P0 is therefore still open. A controlled harness must run the legacy parent
implementation (`ca6c32b2c4492b090d9fa428f44326209f023c15`) and the current
implementation on the same host, measuring the entire process tree and an
explicit serialization counter. It must also explain or fix the current
15.5x increase in the combined evidence/persistence counter before claiming
that the storage change improved campaign throughput.
