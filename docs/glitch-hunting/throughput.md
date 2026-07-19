# Authenticated cold-process throughput

`huntctl benchmark cold-process` measures the current process-per-run harness
without introducing a second execution path. It clones one sealed
`dusklight-harness-run-request/v2` into fresh artifact destinations, invokes the
ordinary authenticated executor sequentially, and writes
`dusklight-cold-process-throughput/v2`.

Each attempt retains its exact request and result identities, terminal,
objective boundary, realized-tape digest, gameplay-trace digest, objective
evidence digest, tick counts, native-process time, end-to-end harness time, and
the exact `dusklight-native-lifecycle-timing/v1` artifact. Native marks cover
process entry, parsed configuration, Aurora initialization, engine readiness,
stage readiness, first/last candidate ticks, proof flush, engine shutdown, and
exit readiness. The summary divides the parent-observed process envelope into
startup, stage loading, simulation, artifact flush, teardown, and unattributed
process-envelope overhead.

The report also seals its host OS, architecture, hardware/CPU model, logical CPU
count, memory, and recording time. It is comparable only when every attempt has
complete artifacts and identical semantic and artifact evidence. A difference
writes the diagnostic report but returns a failing exit status.

## Run and validate

First produce a sealed request with the checked objective conformance command
documented in [objective suites](objective-suites.md). Then choose a new ignored
artifact root and run:

```sh
ROOT=build/benchmarks/cold-process-stage-ready
REQUEST=build/harness/core-conformance-macos/requests/stage-ready-f-sp103/attempt-001.json

cargo run --manifest-path tools/huntctl/Cargo.toml -- \
  benchmark cold-process \
  --request "$REQUEST" \
  --artifact-root "$ROOT" \
  --output "$ROOT/report.json" \
  --repository-root . \
  --repetitions 5

cargo run --manifest-path tools/huntctl/Cargo.toml -- \
  benchmark validate-cold-process --report "$ROOT/report.json"

jq '{host, comparable, summary}' "$ROOT/report.json"
```

Rates and native-process share are stored as integer millionths so sealing and
validation are exact across JSON implementations. Divide those fields by
1,000,000 for candidates/second, ticks/second, or a unit share.

The artifact root must be repository-relative and absent. Neither requests,
results, nor reports are overwritten. Generated evidence remains under ignored
`build/`; recording a baseline does not promote gameplay artifacts into Git.

## First macOS baseline

On 2026-07-19, the checked `stage-ready-f-sp103` request produced five
comparable reached results on a 14-logical-CPU Apple M4 Pro (`Mac16,7`, 48 GiB,
macOS 26.5.1, arm64). The template request digest was
`1a7b1ad2e187fcd3835860d2d420a7bd713c25a5a39a6899c48775a90f29c9d7`;
the sealed report digest was
`8fcc01f948de6473fd56afecb573453b84d105ecc6ba4e79b0cd03d90a411e0f`.
All attempts reached the objective at tick 44, consumed 45 logical ticks, and
agreed on the realized tape, trace, objective evidence, and terminal boundary.

The five runs took 10.231 seconds end to end: median 2.006 seconds, p95 2.200
seconds, 0.489 candidates/second, and 21.992 logical ticks/second. Native
process time accounted for 98.93% of the measured total; validation and other
work outside the launched process accounted for 109.956 milliseconds across
all five attempts.

That v1 report did not separate work inside the native process. It remains the
historical process-per-run baseline, but new reports use the v2 phase contract.

## Phase-attributed macOS baseline

On 2026-07-19, five v2 runs of the same checked stage-ready objective were
again comparable and reached the same tick-44 boundary. The template request
digest was
`7147bddd04333c33c7e1a3bb98aad3c8f6a4c931f81747f64b49508905b02195`;
the report digest was
`89c5c2a50a34d6564c37325cd384bf58943705ce1abc60b23b23a483ccec9b6e`.
Median end-to-end time was 2.209 seconds, p95 was 2.272 seconds, throughput was
0.453 candidates/second, and the parent-observed native process envelope was
98.80% of total time.

Across all five native process envelopes, startup used 3.627 seconds (33.28%),
stage loading 2.914 seconds (26.73%), candidate simulation 1.285 seconds
(11.79%), proof flush 34 milliseconds (0.31%), and teardown 771 milliseconds
(7.07%). The remaining 2.268 seconds (20.81%) was outside the native entry/exit
marks but inside the parent-observed spawn/wait envelope, so it stays explicit
as process-envelope overhead rather than being assigned to engine work.

The first persistent-session implementation should therefore keep the process,
Aurora, DVD host, and process-lifetime services alive while running sequential
stage-boot requests through the unchanged authenticated request/result path.
It must introduce an explicit world teardown/reinitialize seam and prove an
A/B/A request sequence against cold runs before claiming reset equivalence.
Full-memory checkpoints, concurrent sessions, and general arbitrary-state
restore remain out of scope until that narrower reset seam is deterministic.
