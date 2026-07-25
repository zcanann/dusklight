# Tactic-route hot-path before/after benchmark

Date: 2026-07-25

This closes the storage/orchestration P0 benchmark on the sealed Ordon
exit-approach campaign. Machine-readable results are in
`tactic-route-p0-storage-before-after-20260725.json`.

The legacy implementation is
`ca6c32b2c4492b090d9fa428f44326209f023c15`, immediately before binary
checkpoint commit `3256fc212d`. The current implementation is
`3b727bb9c9`, after binary checkpoints, binary decision journals,
content-addressed campaign objects, on-demand graph projection, journal
compaction, and exact resume verification.

The legacy revision was built from a Git archive, not a worktree. Both
revisions used the dev profile, the same host, authenticated native executable,
game data, process-boot fixture, request, execution binding, seed, and tactic
parameters.

## Sealed workload

- Optimization request:
  `routes/Glitch Exhibition/ordon-exit-approach-learning/benchmarks/ordon-exit-approach-tactic-q-discovery-v1.request.json`
  (`296778d879e533f1b50f5eb7e5fbf886e2d0c23f230716c9c91b213973ef45cc`)
- Execution binding:
  `build/campaigns/ordon-exit-approach-second-pair-20260725/execution-v4/execution.json`
  (`47067b4d7a29dfbf9821fa9f2663891023ecc05e30e40cca552fa02f3f77ef9b`)
- Seed: `181081`
- Decision budget: `64`
- Branch interval: `8`
- Refit interval: `1`
- Epsilon per million: `350000`

The common invocation was:

```text
huntctl.exe learn tactic-route
  --repository-root C:\Projects\dusklight
  --request "routes/Glitch Exhibition/ordon-exit-approach-learning/benchmarks/ordon-exit-approach-tactic-q-discovery-v1.request.json"
  --execution build/campaigns/ordon-exit-approach-second-pair-20260725/execution-v4/execution.json
  --output OUTPUT
  --seed 181081
  --decisions-per-seed 64
  --branch-every 8
  --refit-every 1
  --epsilon-per-million 350000
```

The retained outputs are:

- legacy:
  `build/campaigns/ordon-exit-approach-second-pair-20260725/tactic-route-p0-before-replay`;
- current:
  `build/campaigns/ordon-exit-approach-second-pair-20260725/tactic-route-p0-current-v2`.

## Semantic identity

Both runs made four decisions, classified three as useful, reached the terminal
in 89 native ticks, and emitted the same successful tape
`32fd72b0f490d07a82e99329db148cf6d9d2b9dbc512309ce0cbffc49d01dde4`.
Each decision retained the same tactic, accumulated route length, and native
snapshot:

| Decision | Tactic | Accumulated ticks | Result snapshot |
| ---: | --- | ---: | --- |
| 0 | `goal.seek.route.03` | 40 | `4018a2f19b014b7d2e4700b4f401729f0bb8e87ad3730179491d2d6ae16a3db2` |
| 1 | `goal.seek.route.03` | 80 | `2e9641085c25691de783cef5f8cdcae6bc3256ab00f8f4f0eaebe33c6bd79940` |
| 2 | `attack.jump` | 86 | `04b53a8dbae60c29b4b61bd66e76f462f688cbb4011194a6b593c8858a01ff20` |
| 3 | `attack.jump` | 89 | `75faea5689b8bd2f8a59c5f6ece275758eb8a3ece72a5595d8bb36da40f8aabf` |

The checkpoint codec benchmark additionally normalizes only the schema and
format-specific content digest, then requires the complete legacy and current
checkpoint payloads to be equal before timing either codec. Canonical
round-trip bytes must also match both retained files.

## Results

External wall time covers the complete `huntctl` invocation. Peak working set
is the maximum process-tree sum sampled every 250 ms. Bytes and files include
the complete campaign output tree.

| Measurement | Legacy | Current | Change |
| --- | ---: | ---: | ---: |
| Files written | 274 | 342 | +24.8% |
| Bytes written | 13,774,067 | 10,539,588 | -23.5% |
| Operational learner-state bytes | 2,841,814 | 87,817 | -96.9% |
| Checkpoint-root serialization/iteration | 69.326398 ms | 1.506329 ms | 46.0x faster |
| Checkpoint-root encoded bytes | 2,841,814 | 4,117 | -99.9% |
| Complete process wall time | 51.293214 s | 52.188857 s | +1.7% |
| Useful decisions/s | 0.073968 | 0.071251 | -3.7% |
| Native ticks/s | 2.194410 | 2.113793 | -3.7% |
| Peak process-tree working set | 812,511,232 B | 792,666,112 B | -2.4% |
| Runner seed wall time | 40.557593 s | 42.104398 s | +3.8% |
| Native simulation | 39.972153 s | 41.240266 s | +3.2% |
| Evidence projection and persistence | 0.118755 s | 0.432475 s | +264.2% |

Serialization was measured for 100 iterations with:

```text
huntctl.exe learn benchmark-tactic-checkpoint-codecs
  --legacy-json-checkpoint LEGACY_CHECKPOINT.json
  --current-checkpoint CURRENT_CHECKPOINT.dtqz
  --iterations 100
```

The legacy total was `6,932,639,800 ns`; the current total was
`150,632,900 ns`. This codec boundary measures deterministic checkpoint-root
serialization only. The current root is a manifest referencing immutable
content objects, while the legacy root embeds the entire campaign. Object-store
serialization, journal encoding, filesystem sync, and readable projections
remain included in the separate complete-process and combined
evidence/persistence measurements.

The current tree stores 4,117 checkpoint-manifest bytes, 82,278 immutable
content-object bytes, and 1,422 decision-journal bytes. The remaining campaign
bytes are retained proof/reporting objects and native scratch. File count rises
because the current store separates immutable objects by digest; the
operational learner state nevertheless shrinks by 96.9%.

The broader persistence phase regresses by 314 ms on this tiny campaign. That
regression is real, but the phase is 1.03% of seed wall time and is not the
throughput bottleneck. Native execution consumes 41.65 of 42.10 seed seconds.
The benchmark therefore does not claim an end-to-end speedup; it demonstrates
that serialization and persistence no longer dominate and identifies native
worker utilization as the next bottleneck.

## Resume and cold-replay gates

The exact-resume campaign test continues an uninterrupted campaign and a
disk-loaded binary checkpoint through the same next selection and terminal
transition. It compares model CBOR, graph projection, sampled frontier, route
tape, checkpoint payload, and final result.

The shared successful tape was replayed twice from cold process boot with no
controller or model in the loop. Both repetitions reached
`ordon_spring_load_committed` at simulation tick `708`, tape frame `708`, with
boundary fingerprint `219dfdb648caacd91bbba800c09b4b1e` under
`dusklight.milestone-boundary/v6`. The retained proof is:

`build/campaigns/ordon-exit-approach-second-pair-20260725/tactic-route-p0-cold-replay-proof-20260725/cold-replay.proof.json`

## Conclusion

Operational tactic-Q checkpoints and decision journals are binary. The sealed
campaign preserves exact decisions, resume state, and final replay identity.
Checkpoint-root serialization is 46.0 times faster, operational learner state
is 96.9% smaller, and persistence is a measured 1.03% of seed wall time.
End-to-end throughput regresses 3.7% on this sample because native simulation
is 3.2% slower; no storage speedup is inferred from that noise.
