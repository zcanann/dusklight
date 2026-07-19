# Authenticated cold-process throughput

`huntctl benchmark cold-process` measures the current process-per-run harness
without introducing a second execution path. It clones one sealed
`dusklight-harness-run-request/v2` into fresh artifact destinations, invokes the
ordinary authenticated executor sequentially, and writes
`dusklight-cold-process-throughput/v1`.

Each attempt retains its exact request and result identities, terminal,
objective boundary, realized-tape digest, gameplay-trace digest, objective
evidence digest, tick counts, native-process time, and end-to-end harness time.
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

This result does not separate executable initialization, stage loading,
simulation, native trace writing, or shutdown inside that 98.93%. It therefore
justifies measuring a persistent-session prototype, but does not prove that
soft reset is safe or predict its speedup. Any persistent or reset benchmark
must consume an equivalent sealed request and retain the same terminal,
boundary, tape, trace, evidence, and tick identities before its throughput can
be compared with this baseline.
