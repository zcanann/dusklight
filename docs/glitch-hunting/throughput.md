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
`success: false`, so none is a valid route improvement or proof promotion. The
best 2.7813-unit near miss is checked in only as the unproved
`to_ordon_spring_near_miss_2p781` workbench sibling for visual review. The proof
is blocked pending a measured non-local proposal family; repeating the
exhausted local neighborhoods would not satisfy the gate.

A bounded non-local follow-up used the existing native reactive controller to
derive world-coordinate waypoint policies from the successful incumbent trace.
Six cold 125-tick policies varied waypoint interval, phase, lookahead, and
terminal extension while retaining the seven roll edges. Every run reproduced
source fingerprint `ac7c32788fc3b5c59046386d95b9b5b4`, none reached the
terminal predicate, and the best endpoint remained 66.6412 signed units from
the exit edge. Because that is far worse than the retained 2.7813-unit exact
input near miss, the family was rejected without new controller code.

## Simulation-only frame sink

Headless farming now retains the ordinary CPU draw traversal and GX FIFO drain,
but discards each completed renderer packet before per-frame pipeline lookup,
GPU command encoding, and queue submission. The prior Dawn null-backend path is
available through `--headless-submit-gpu-frames` only as an audit comparator.
Suffix-batch timing artifacts report per-batch and process-total command-buffer
submissions, direct queue writes, shader modules, render/compute pipelines and
discarded frames explicitly; absent authenticated GPU timestamps remain `null`
rather than being reported as zero time.

A Windows discard/submit/discard proof on 2026-07-20 ran the same two 125-tick
Ordon candidates from the same 440-tick source boundary. Each discard process
reported zero command-buffer submissions, zero direct queue writes, zero
shader modules, zero render/compute pipelines, and 250 discarded candidate
frames. The null-backend comparator reported 690 process submissions (440
prefix plus 250 candidate frames), 154 shader modules, 155 render pipelines
and zero discarded frames. Across the three runs, the six authenticated native
episode payloads formed two
duplicate-trajectory groups with three byte-identical copies each and no
determinism conflict. The complete observation stream included 1,500 samples,
47-48 actors per sample, 24-25 dynamic colliders, six player collision
surfaces, and no truncated actor observations.

This is a renderer-work invariant, not a throughput win claim. The final three
batch wall times were 6.053, 6.015, and 5.896 seconds; state validation dominated
this particular two-candidate run, and the Dawn null backend made renderer
removal smaller than run-to-run noise. The farming path still uses a hidden SDL
window as an event/size anchor and creates a Dawn null device plus metadata
resources. It does not initialize an SDL/ImGui renderer backend, create shader
or pipeline objects, present, or issue queue operations.

## Single-worker suffix profile

A normal-fidelity 128-candidate Windows batch on 2026-07-20 measured the
routine farming path with state-hash verification disabled. It completed all
128 125-tick episodes: 16,000 transitions in 24.966 seconds, or 640.87
transitions/second and 5.13 episodes/second. Checkpoint restore used 2.504
seconds across 127 restores (19.71 milliseconds each). Inclusive simulation
used 10.229 seconds, observation capture used 0.354 seconds, and corpus encoding
used 1.165 seconds. The 526,802,688 uncompressed episode bytes became 9,212,650
compressed bytes, or 575.79 compressed bytes per transition. Policy application
used 1.943 milliseconds; learned policy inference was not present and is not
reported as zero. The GPU sink reported no code creation or queue operations.

Inside the inclusive simulation boundary, actor draw traversal accounted for
1.596 seconds and the CPU painter for 4.694 seconds. An audit build then tried
suppressing each boundary independently and together from process start. The
ordinary control verified the exact frame-440 `gameplay-ready-f-sp103` source
fingerprint. Every suppression mode reached that frame with no stage and no
player, so it failed before a suffix episode could begin. These callbacks are
therefore simulation-relevant despite their graphics-facing names. The audit
switches were removed; this result is evidence against the optimization, not a
new fidelity mode.

The batch request SHA-256 was
`95afa6e2af879e994c7780c724a0d81c81b8d54c7b2f90f9faba2fb4c3a0a54b`;
the result SHA-256 was
`bf517ee05b8349de78dc6d87a86790d659203984f405401fd4099865bd492e46`;
and the native episode-shard SHA-256 was
`e3f4e1c4870419f7f558e7d2d45a249684522d1909010e7ad44ce8e8b08e9aae`.

## Persistent-worker scaling

A same-host weak-scaling sweep on 2026-07-21 measured one isolated process and
one persistent frame-440 source checkpoint per worker. Every worker evaluated
the same 111 candidates for 125 ticks each with state-hash verification
disabled. All 27 workers and 2,997 episodes passed the strict
`gameplay-ready-f-sp103` source boundary; every result reported zero shader or
pipeline creation and zero GPU queue operations. The current Debug executable
was used throughout, so these absolute rates are not directly comparable to
the Windows single-worker profile above.

| Workers | Transitions | Batch seconds | Transitions/s | Episodes/s | Efficiency | Checkpoints | Restore mean | Observation mean |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 13,875 | 99.286 | 139.75 | 1.12 | 100.0% | 0.27 GiB | 4.92 ms | 28.54 us |
| 2 | 27,750 | 106.005 | 261.78 | 2.09 | 93.7% | 0.55 GiB | 5.24 ms | 32.95 us |
| 4 | 55,500 | 102.576 | 541.06 | 4.33 | 96.8% | 1.10 GiB | 6.10 ms | 32.87 us |
| 8 | 111,000 | 108.057 | 1,027.23 | 8.22 | 91.9% | 2.20 GiB | 6.13 ms | 40.86 us |
| 12 | 166,500 | 123.428 | 1,348.97 | 10.79 | 80.4% | 3.29 GiB | 6.92 ms | 63.04 us |

The eight-worker run consumed 841.56 process CPU-seconds in 113.75 outer
seconds, or 7.40 CPU-core equivalents, and retained six logical CPUs of host
headroom. Twelve workers consumed 11.20 core equivalents and added 31.3%
throughput for 50% more processes while observation latency and restore cost
rose. Eight workers are therefore the default on this 14-logical-CPU host;
twelve is a tested dedicated-host ceiling, not the interactive default. Each
worker has its own card, state and cache roots, so a process crash is isolated
to that worker's 111-episode shard. Checkpoint memory is 294,694,644 bytes per
worker and is not the limiting resource on this 48 GiB host.

Corpus density was identical in every trial at 598.97 compressed bytes per
transition. Policy inference remained `not_present`; this comparison does not
misreport it as zero. The sealed aggregate report is
[`benchmarks/macos-worker-scaling-20260721.json`](benchmarks/macos-worker-scaling-20260721.json),
SHA-256 `b00148589ff5e7cabe256d6852dd0e38cecc64474ab033cd37ba92e11d251e5e`.

## Fixed-work graph-expansion scaling

On 2026-07-29, a balanced two-repetition macOS sweep measured the complete
native tactic path at 1, 2, 4, 8, and 16 persistent workers. Every cell
completed one learner decision and the same 16 graph-authoritative executable
expansions. The worker order was ascending in the first repetition and
descending in the second to expose host-temperature and ordering bias.

| Workers | Median seconds | Useful expansions/s | Speedup | Efficiency |
| ---: | ---: | ---: | ---: | ---: |
| 1 | 103.322 | 0.155 | 1.00x | 100.0% |
| 2 | 80.412 | 0.199 | 1.28x | 64.2% |
| 4 | 70.952 | 0.226 | 1.46x | 36.4% |
| 8 | 65.722 | 0.243 | 1.57x | 19.7% |
| 16 | 85.211 | 0.188 | 1.21x | 7.6% |

All ten runs reported zero replay staleness. Their serialized state-graph
digests differ because worker scheduling changes lease and admission history,
but a post-run audit of the completed executable expansion identities, targets,
route checkpoints, and replay-evidence identities produced the same digest for
every checkpoint:
`8bc738c30fbd903dc0c62799c304d809edb9797a99a529bb861ba336e6f4e432`.
The throughput report now compares this semantic work-set identity while
retaining the full state-graph digests for scheduling audit.

Useful execution time scaled from 45.145 seconds at one worker to 3.823 seconds
at sixteen. Process launch and native boot instead consumed 54.355-58.226
seconds through eight workers and 78.438-80.713 seconds at sixteen. The
fourteen-logical-CPU host therefore saturates after eight workers, and process
startup dominates every cell. The next fixed-work treatment is persistent
process reuse across campaign cells or elimination of redundant native boot;
additional worker capacity is not justified by this curve.

The ignored evidence root is
`build/benchmarks/ordon-native-tactic-fixed-work-scaling-v1`. Its sealed report
content SHA-256 is
`916bb1bd02827c590ccef1cfc849d30d19968c3928a75c03f4d76ca61fb2a579`;
the optimization request, execution binding, and execution-plan identities are
`d37c1635757049582b7fa571bf2d45b6d7be88633d4a32a42502413c3ebbbf75`,
`e4def9095daf4500800647966e9e01b527a0af91e02f4ba88fb22dde3aee85cb`,
and `c3896c9163d2aea867b63f0bdb44a3e7f9a96a98b315c57db03f95c4befe24fe`.

### Persistent-fleet treatment

The v2 treatment on 2026-07-29 moved native worker ownership above individual
route runs. One authenticated 16-process fleet now remains alive while each
curve cell selects a bounded active prefix. Every submitted batch still
restores and validates the complete typed source boundary; the optimization
request, execution binding, terminal predicate, initial facts, and root
checkpoint must match the fleet before a route run is admitted.

The fleet took 79.465 seconds to launch once. The ten measured samples then
reported exactly zero process-launch time:

| Active workers | Median seconds | Useful expansions/s | Warm speedup | Efficiency |
| ---: | ---: | ---: | ---: | ---: |
| 1 | 45.478 | 0.352 | 1.00x | 100.0% |
| 2 | 24.014 | 0.666 | 1.89x | 94.7% |
| 4 | 13.010 | 1.230 | 3.50x | 87.4% |
| 8 | 7.386 | 2.166 | 6.16x | 77.0% |
| 16 | 5.485 | 2.917 | 8.29x | 51.8% |

The same-topology 16-worker fixed target fell from the cold curve's 85.211
seconds to 5.485 seconds, a 15.54x steady-state reduction. The complete
balanced curve consumed 190.747 seconds of measured work and 270.212 seconds
including its one fleet boot, versus 811.235 seconds when all ten cells booted
their own pools. Startup is therefore amortized and retained as a separate
top-level measurement, not removed from the evidence.

All ten cells completed the same 16 graph-authoritative expansions, agreed on
semantic expansion/evidence digest
`8bc738c30fbd903dc0c62799c304d809edb9797a99a529bb861ba336e6f4e432`,
had zero replay staleness, remained inside the checkpoint-memory bound, and
increased useful throughput at every worker count. The v2 report rejects any
sample with nonzero process-launch time, so a regression to per-cell boot
cannot pass as persistent reuse.

This remains a warm-fleet microbenchmark, not a long-campaign scaling result.
Throughput report v4 closes the old pass-condition hole: a curve now requires
at least 16 decisions per cell, at least two learner updates and published
models, durable replay, repeated non-root restores, graph admission,
content projection, persistence, and observed bounded-cache eviction in every
sample. Each sample retains those counters plus selection, branching, worker
wait, native occupancy, IPC, preparation, learner, admission, persistence, and
orchestration phase times. Native worker occupancy is derived against
`wall * active_workers`, so additional processes cannot be credited for raw
native work that did not occupy them. The historical one-decision measurements
above cannot validate as v4 evidence.

Throughput report v4 carries forward v3's corrected staleness accounting: the
number of fitted snapshots a lane skipped before refreshing is pressure, not
model lag. The gate uses the fitted model's actual lag behind durable replay.
Native worker cache capacity is derived from the sealed aggregate memory budget
instead of always requesting 640 MiB per worker; scratch campaign audit v2
recomputes both bounds from the route and execution plan.

The ignored evidence root is
`build/benchmarks/ordon-native-tactic-fixed-work-persistent-v2`; its sealed
report content SHA-256 is
`1499e563b4fb308b005e88c0fb7ff60df283edce4bfe9c47c37d0755cf656aaa`.

### Compact persistent-batch transport

Authored worker launches remain JSON so their source contract is directly
inspectable. Repeated tactic decisions now use the versioned `DSKSBX` binary
envelope. Its header binds the exact payload length and canonical XXH3-128
digest; its bounded payload carries source and checkpoint identities as raw
128-bit content hashes, controller programs as raw bytes, and PAD inputs as
run-length encoded fixed-width states. Unknown flags, invalid bounds, duplicate
candidate identities, malformed controllers, trailing bytes, and payload
tampering fail closed in the native parser.

The envelope is transport only. Rust retains the full typed request in memory,
validates it against the authenticated persistent-worker identity before
submission, and uses that same request to attach the result. Native transition
trajectories continue to cross the boundary as compact `.dseps` episode shards;
the result JSON contains report metadata and content references rather than a
second transition blob.

The 2026-07-29 macOS smoke at
`build/benchmarks/compact-suffix-native-smoke-v2` executed one learned
graph-authoritative proposal for 16 native ticks through a freshly sealed
persistent worker. The hot request is 180 bytes, versus 1,878 bytes for the
same candidate under the JSON transport: a 10.43x reduction, or 90.4% fewer
request bytes. Both runs bind candidate
`02d347226cc9d75c131e3b62a7981acb7eb50a731802b62d8ebbe7b8d782a17a`;
the smoke report file SHA-256 is
`7948cb4938345e38cf1c71b270f2540a42d9568d083b2fb401b7498eb73558d9`.

### Native subsystem suppression parity

The native farming path now exposes typed audit comparators for GPU frame
submission, CPU renderer submission, presentation, ImGui frames, host pacing,
and the host audio device. The production path keeps deterministic game audio
emulation, game audio updates, and gameplay draw traversal active. A seventh
paired treatment disables state-hash proof while leaving the production
runtime configuration unchanged.

These switches have real dependencies, so the benchmark does not construct
invalid combinations. GPU submission is added to the production-suppressed
reference before presentation; ImGui is added after presentation; and CPU
renderer submission is added after ImGui. Host pacing and host audio each
compare directly with production suppression. An all-retained composite
provides a final interaction control. The headless SDL window remains hidden
for every comparator, including retained presentation.

The 2026-07-29 macOS arm64 run at
`build/benchmarks/native-subsystem-parity-macos-v1` evaluated one exact
16-tick candidate from source frame 506 in nine conditions. Every condition
produced native episode payload XXH3-128
`b544f99fadd0781a21c81c04d90371f2`, terminal evidence SHA-256
`366e88a4d24fe9649424886ce3bb91ed32e3e5dbb985ba03d2f0ae910073ef3f`,
source boundary `e7ac8251329f22a5df682bbe5eb2a2ba`, and terminal boundary
`1ec4b91fd43ce914c4e91ed6f216dc1a`. Configuration audits confirmed zero GPU
submissions and discarded frames on the production path, nonzero null-backend
submissions in retained comparators, zero CPU renderer time when suppressed,
nonzero CPU renderer time when retained, and the requested pacing/audio states.

All proof-enabled treatments also passed 16 samples of full-state validation
inside their native process. Their raw state-proof digests are retained as
diagnostics but are not compared across separately booted address spaces,
because they include process-local pointers and allocator state. Cross-process
parity instead requires identical authenticated episode bytes, simulated
ticks, source and terminal boundaries, and terminal observations/evidence.
The proof-disabled follow-up matched all of those identities and reported state
validation as disabled.

The report schema is `dusklight-native-subsystem-parity/v1`; the ignored report
path is `build/benchmarks/native-subsystem-parity-macos-v1.report.json`, and its
content SHA-256 is
`69ef05bc1d62b3704e057fa3fc848260f684ea79bf0ac0762ddeb1548d42acb8`.
