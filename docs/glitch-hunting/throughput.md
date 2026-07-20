# Authenticated cold-process throughput

`huntctl benchmark cold-process` measures the current process-per-run harness
without introducing a second execution path. It clones one sealed
`dusklight-harness-run-request/v2` into fresh artifact destinations, invokes the
ordinary authenticated executor sequentially, and writes
`dusklight-cold-process-throughput/v2`.

Each attempt retains its exact request and result identities, terminal,
objective boundary, realized-tape digest, gameplay-trace digest, objective
evidence digest, tick counts, native-process time, end-to-end harness time, and
the exact native lifecycle artifact. Current runs emit
`dusklight-native-lifecycle-timing/v3`, which adds process CPU time to the v2
authenticated post-run session-reuse audit; historical v1 and v2 timing remain
valid. Native marks cover
process entry, parsed configuration, Aurora initialization, engine readiness,
stage readiness, first/last candidate ticks, proof flush, engine shutdown, and
exit readiness. The summary divides the parent-observed process envelope into
startup, stage loading, simulation, artifact flush, teardown, and unattributed
process-envelope overhead. V3 summaries also report process launches, declared
prefix and candidate ticks, candidate-tick throughput, native CPU utilization,
simulator idle time, and generated artifact files and bytes.

The report also seals its host OS, architecture, hardware/CPU model, logical CPU
count, memory, and recording time. It is comparable only when every attempt has
complete artifacts and identical semantic and artifact evidence. A difference
writes the diagnostic report but returns a failing exit status.

## Run and validate

For a checked route segment, `route-cold-process` derives the absolute tape,
parent-prefix length, objective program, exact observation dependencies, and
sealed request directly from the timeline. The timeline's sibling support
directory supplies `benchmarks/process_boot.fixture.json` and
`benchmarks/<goal>.observation.json`:

```powershell
tools\huntctl\target\debug\huntctl.exe benchmark route-cold-process `
  --timeline "routes\Glitch Exhibition\intro.timeline" `
  --segment to_ordon_spring_q125 `
  --goal ordon_spring_load_committed `
  --game "build\windows-clang-relwithdebinfo\dusklight.exe" `
  --dvd "orig\GZ2E01\Legend of Zelda, The - Twilight Princess (USA).iso" `
  --artifact-root "build/benchmarks/ordon-q125-cold-v3" `
  --repetitions 5
```

The command refuses an existing artifact or generated-input root and defaults
the report to `<artifact-root>/report.json`. Generated evidence stays ignored
under `build/`.

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

## Ordon route Windows baseline

On 2026-07-19, two v3 cold runs materialized directly from the checked `intro`
timeline both reached `ordon_spring_load_committed` at absolute tick 565. Both
runs agreed on realized input
`fad64d9c0610ef954a9f18c348d663dd249ae14d54a164471e766a2d38107f6f`,
gameplay trace
`95bc7caa5f5ba412415d4e8947ae5585cb54b370e315f1df64af22100d41c798`,
objective evidence
`427d5f7b732cc07a35685ec506c1e1ec948bd97558e0416be9eac65730a4d9a1`,
and terminal fingerprint `5f0db2cd758da9d20bffd7c7def9af70`. The sealed
report digest was
`83a451ffd40b44197155a07e57a214c2760fa2548830935b0d9d1c848dd701d8`.

The parent prefix consumed 440 ticks. The segment score is 125 ticks
(`565 - 440`); the benchmark counts 126 sampled suffix ticks because it includes
both the source-adjacent sample and the terminal sample. Across both launches it
measured 252 candidate ticks in 3.778 seconds, or 66.708 candidate ticks/second.
It launched two processes, spent 2.194 seconds outside candidate simulation,
used 3.688 process CPU-seconds across engine threads (97.61% of elapsed time),
and produced 34 files totaling 4,044,882 bytes. These numbers quantify why
process reuse and prefix checkpoint
restore are the next gate; they are not included in the route score.

## Route diagnostics

Every harness execution requests all gameplay-trace channels. The Ordon
baseline contains applied pads, Link motion, camera, player action/animation,
background collision, and goal progress on all 566 records. A concise report
for the candidate suffix is generated without rerunning the game:

```powershell
tools\huntctl\target\debug\huntctl.exe trace route-diagnostics `
  "build\benchmarks\ordon-q125-cold-v3\attempt-001\gameplay.trace" `
  --source-boundary-frame 439 `
  --terminal-frame 565
```

The analyzer requires a complete trace and all diagnostic channels. It reports
horizontal path and direct distances; their excess and ratio; facing-to-motion
angular error; collision displacement loss and correction; configurable
per-tick yaw-change episodes; and front-roll cycles reconstructed from the
procedure plus primary-animation resets. These are diagnostics, not alternate
success or scoring authorities.

For the current 125-score winner it measured 3,641.556 horizontal path units versus
3,062.366 direct units (579.189 excess, 84.10% direct/path efficiency), 0.649°
mean facing-to-motion error, four collision-correction frames with 6.370 units
of horizontal loss, and a longest seven-tick turn episode at the default
512-s16 threshold. Roll starts occurred at frames 440, 460, 480, 500, 520, 540,
and 560: exact 20-tick spacing with each A edge applied on the reconstructed
roll-start frame.

## Sub-125 suffix search outcome

On 2026-07-20, the checkpointed native suffix loop retained 73 passed search
batches totaling 18,867 candidates and 2,358,375 simulated suffix ticks. None
reached the `F_SP103` to `F_SP104` terminal predicate within the 125-tick batch
limit. Median trusted restore time in the later large batches stayed near 4.6
milliseconds; for example, the 1,000-candidate independent stick-component
batch reported 4,646 microseconds.

The equal-budget comparison assigned 107 candidates and 13,375 simulated ticks
each to deletion, earliest button-edge, heading, corner, timing, and structured
ranked proposals. Later batches exhausted all unique single-frame deletions,
fixed-length delete-and-hold shifts, the neighboring 18–22 tick roll cadences,
fine and broad heading changes, duration copies, corner and collision windows,
long path windows, symmetric and
asymmetric lane shifts, magnitude/clipping changes, measured pair/triple
combinations, and 1,000 independent X/Y stick-component edits.

Geometry remained diagnostic only. It guided the failed-route lineage from an
approximately 11.8-unit signed exit-edge miss to 4.0807 after path/corner
repairs, 3.0181 after a measured lane shift, 2.9324 after a local clipped-stick
change, and 2.7813 after fine ranked combinations. The native terminal result
remained authoritative throughout: every retained candidate still reported
`success: false`, so none is a valid route improvement and no tape was
promoted. The proof is blocked pending a measured non-local proposal family;
repeating the exhausted local neighborhoods would not satisfy the gate.
