# Testing and visual TAS playback

## Tests

The retired PowerShell aggregate runner intentionally has no replacement.
Run the owned test surface directly so failures stay attached to the component
that produced them:

```sh
python3 tests/automation_boundary_test.py
cargo test --manifest-path tools/huntctl/Cargo.toml
cargo fmt --manifest-path tools/huntctl/Cargo.toml --check
cargo clippy --manifest-path tools/huntctl/Cargo.toml --all-targets -- -D warnings
cmake --build --preset macos-default-debug
ctest --test-dir build/macos-default-debug --output-on-failure
```

Use the configured Windows preset in the last two commands on Windows. Native
targets can also be built and run individually when isolating a failure. Search
and replay commands should use a fresh directory below `build/automation-state`
so they never write the normal Dusklight profile.

## Visual route playback

For route work, choose the single **Glitch Hunt: Route Workbench** entry under
**Run and Debug** and press F5. VS Code builds Dusklight and the Rust workbench,
then opens a browser containing the checked-in segment tree. Every playable
segment and standalone Project tape has exactly two launch choices. **Playback**
uses the global Playback pacing and renders the complete boot-rooted tape.
**Resume (accelerated)** runs that same complete tape windowless, muted,
presentation-suppressed, and uncapped, then reveals its retained terminal frame
and releases live input at normal pacing. It works for root segments as well as
children. Recording
pacing is configured separately and begins only after the same exact fast
handoff. These are host-pacing controls only: all modes use absolute 30 Hz
input frames and own all controller ports until handoff.

The Workspace sidebar is a direct view of the Git-owned `routes/` directory.
Folders group route timelines and standalone QA, canary, sample, glitch, and map
boot tapes without a second catalog to maintain. A directory with the same stem
as a `.timeline` is that route's private segment storage and is folded into the
route graph rather than duplicated as standalone entries. Rust tests scan and
compile every visible `.tas`/`.tape` entry. Boot Properties expose an explicit,
opt-in override for process or stage boot, including map, room, spawn, layer,
save slot, and optional fixture/loadout data; disabling it preserves the
artifact's authored boot unchanged.

See [Route segment trees and Git](timelines.md) for lineage semantics.

## Direct tape playback

The Route Workbench is the maintained visual surface. For a standalone tape,
use `huntctl tape run`; add `--headful` for a visible fixed-step run:

```sh
cargo run --manifest-path tools/huntctl/Cargo.toml -- tape run build/my-test.tape \
  --game build/macos-default-debug/dusklight \
  --dvd orig/GZ2E01/GZ2E01.iso \
  --state-root build/automation-state/my-test \
  --headful
```

On Windows, select the Windows build executable instead. Direct automated
launches are strict: the DVD, tape, and state root must all be valid. While a
tape owns the controller ports, host input is quarantined from gameplay; normal
window lifecycle events remain available.

## Deterministic visual shader warmup

Visual playback from the route workbench keeps a persistent renderer cache at
`<workbench-state>/renderer-cache`. Gameplay configuration, memory cards, and
recording state remain isolated per launch; only Aurora's exact GX pipeline
manifest and Dawn's backend-compiled shader cache are shared.

On a fixed-step visual automation run, Dusklight does not admit simulation
ticks or emulated VI retraces while previously learned pipelines are still
compiling. A pipeline first encountered later in the run is compiled
synchronously before its draw is accepted. This can pause wall-clock playback,
but it cannot advance game time or display a frame with that draw omitted.
Headless/null-renderer searches do not pay this cost.

The cache learns incrementally. The first playback of a new route still has to
compile its new pipeline configurations; later playbacks precompile those
known configurations before the tape begins. `huntctl tape run` creates the
isolated state and adjacent renderer-cache roots for direct playback.

## Authoring a custom tape

Copy the smoke `.tas` fixture, define reusable controller states, add exact
frames/repeats/cycles, and compile it:

```sh
cargo run --manifest-path tools/huntctl/Cargo.toml -- tape compile \
  my-test.tas build/my-test.tape
```

Inspect the expanded controller frames before playback with:

```sh
cargo run --manifest-path tools/huntctl/Cargo.toml -- tape inspect \
  build/my-test.tape --frames
```

Use `tape run` as shown above for both visible and headless playback; headless
is the default. Each run should receive a distinct state root.

Custom tapes can be imported and evaluated against the native direct-stage
next-map goal without a scenario wrapper:

```sh
cargo run --manifest-path tools/huntctl/Cargo.toml -- search import-tape \
  --segment fsp103_to_fsp104 --tape build/experiment-42.tape \
  --output build/experiment-42.candidate.json
cargo run --manifest-path tools/huntctl/Cargo.toml -- search seed \
  --segment fsp103_to_fsp104 --candidate build/experiment-42.candidate.json \
  --output build/experiment-42.population --size 1
cargo run --release --manifest-path tools/huntctl/Cargo.toml -- search evaluate \
  --population build/experiment-42.population/manifest.json \
  --game build/macos-default-debug/dusklight \
  --dvd orig/GZ2E01/GZ2E01.iso \
  --output build/experiment-42.evaluation --workers 1 --repetitions 3
```

The evaluator launches isolated writable profiles, evaluates native
memory-backed milestones, records a compact diagnostic gameplay trace, and
writes evaluation artifacts below the requested output root. A movement sequence that misses the goal is
a successful evaluation with `success: false`; setup, launch, or native-result
failures use `evaluation_status: worker_error`. Trace decoding is diagnostic
and cannot change success or score. This distinction lets an optimizer treat
bad candidates as samples without hiding broken workers.

For expensive diagnostic channels, retain bounded windows instead of every
tick:

```sh
dusklight --dvd game.iso --input-tape candidate.tape --exit-after-tape \
  --gameplay-trace build/candidate.trace --gameplay-trace-channels all \
  --gameplay-trace-retention 90,30 --gameplay-trace-retention-capacity 4096 \
  --gameplay-trace-triggers contact,flag,predicate,crash
```

The recorder reserves its output and pre-trigger ring before the game loop.
Untriggered ticks only replace a ring slot; a trigger flushes the preceding 90
ticks, retains the trigger tick and 30 following ticks, and later windows remain
bounded by the declared capacity. The `crash` trigger is an explicit hook for
controlled failure paths that can still perform normal artifact shutdown; an
uncatchable process or host failure cannot be claimed as a completed trace.
Dense and retained output share a hard 131,072-record ceiling. A larger dense
request is rejected before allocation; choose a retention window and capacity
within that bound for long candidates.

The checked seed tape declares its F_SP103 stage origin directly. Dusklight
holds its ports neutral while the stage loads and begins frame zero only after
the exact room, entrance, layer, and live-player readiness check. Its initial
neutral frames now cover only the short automatic opening event; stage loading
is not authored timing slack. An optional save slot is part of the stage-boot
identity and maps to the existing native save-loading path.
