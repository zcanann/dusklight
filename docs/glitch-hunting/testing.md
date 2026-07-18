# Testing and visual TAS playback

## VS Code test selector

Run **Tasks: Run Test Task** from the command palette and choose
**Glitch Hunt: Run Selected Test**. VS Code presents one fixed selector:

- `all`: native automation, Aurora card/time, Rust tests/lint, and a real
  two-worker smoke run;
- `native`: every game-data-free C++ and Aurora test;
- `input-tape`, `gameplay-trace`, `game-clock`, `name-entry`,
  `name-entry-trace`, `rng`,
  `eye-shredder-oracle`, `aurora-card`, or `aurora-time`: one native suite;
- `eye-shredder`: build and run the real isolated Eye Shredder tape three times
  silently and headlessly against the last configured GCN USA image;
- `intro-first-exit`: run the normal-file first-loading-trigger golf three
  times with semantic gameplay traces;
- `intro-cutscene`: continue the route through the fence into `demo01_04`;
- `fsp103-next-map-seed`: start directly in `F_SP103`, evaluate the checked
  route seed against the memory-backed `F_SP104` point-0 goal, and retain its
  trace/result JSON;
- `rust` or `rust-lint`: the control-plane tests or formatting/Clippy; and
- `worker-smoke` or `pool-smoke`: real native process protocol tests.

The same selector works without VS Code:

```powershell
.\tools\glitch-hunting\run-tests.ps1 -Test all
.\tools\glitch-hunting\run-tests.ps1 -Test input-tape
.\tools\glitch-hunting\run-tests.ps1 -Test pool-smoke
```

The two intro scenarios currently double as cold timing-leak probes. Their
input is absolute and branch-free, but the unpaced PC title event can remain
active past the checked menu press while host work is outstanding. A failure
preserves its isolated writable state and trace under `build/test-results`; it
must not be hidden with a reactive tape wait. See the
[intro route benchmark](benchmarks/intro-route.md) for the current boundary.

The runner configures `windows-clang-debug` with code mods disabled and the
fork-only read-only observer boundary explicitly enabled. This keeps the
automation tests usable from ordinary PowerShell rather than requiring a
Visual Studio developer shell, without silently enabling observers in normal
builds.

## Visual route playback

For route work, choose the single **Glitch Hunt: Route Workbench** entry under
**Run and Debug** and press F5. VS Code builds Dusklight and the Rust workbench,
then opens a browser containing the checked-in segment tree. Every playable
segment has **Play from boot**; every non-root segment also has **Play from
parent** and **Play from parent (fast)**. Ordinary parent playback is visible
and uses the global Playback pacing. Fast playback runs the exact complete tape
windowless, muted, presentation-suppressed, and uncapped, then reveals its
retained terminal frame and releases live input at normal pacing. Recording
pacing is configured separately and begins only after the same exact fast
handoff. These are host-pacing controls only: all modes use absolute 30 Hz
input frames and own all controller ports until handoff.

The workbench replaces per-scenario launch configurations and generated-winner
dropdown entries. Git-owned segments and repeat-proved generated search results
appear in the same graph; generated nodes remain uncommitted and restricted
until their tape and proof are promoted into `routes/`.

See [Route segment trees and Git](timelines.md) for lineage semantics.

## Legacy named playback

For checked Eye Shredder playback, run **Tasks: Run Task**, choose **Glitch
Hunt: Play Visual Scenario**, then select `eye-shredder`. It resolves the last
manually selected image, builds the fixture, runs the memory, renderer,
opening-event, and playable-gameplay oracle, and preserves the result/trace
under `build/test-results/eye-shredder`.
Playback owns all four controller ports and consumes only absolute frames. DVD
and memory-card work is dispatched synchronously for fixed-step tape runs; any
deviation from the checked simulation ticks or complete trace hash fails
instead of delaying or retrying input. After the 650-frame tape reaches normal
`F_SP103` gameplay, automation releases the ports and live controller input is
restored. Aurora's XF=12/BP=4 shredded diagnostic remains synthetic;
pixel-perfect console raster corruption is not claimed.

1. Put a supported Twilight Princess disc image somewhere on your machine. A
   repository-local image such as `orig/GZ2E01/GZ2E01.iso` remains ignored by
   Git.
2. Run **Tasks: Run Task** and choose **Glitch Hunt: Play Visual Scenario**.
3. Choose `eye-shredder`, `intro-first-exit`, `intro-cutscene`, or
   `fsp103-next-map-seed` from the fixed dropdown. The next-map seed uses a
   direct `F_SP103,1,1,3` stage launch and is the visual route-search baseline.
   Eye Shredder is the default; `boot-start-smoke` is only a short
   boot/menu diagnostic. Leaving the DVD
   prompt blank uses the last image selected through Dusklight's Browse screen.

Search winners have two fixed playback handoff names in the same dropdown:
`boot-search-champion` plays `build/boot-search-champion.tape` from a cold boot;
`route-search-champion` plays `build/route-search-champion.tape` from
`F_SP103,1,1,3`. Search promotion writes those generated files under the
already-ignored `build/` tree. Selecting one before its tape exists reports
that the corresponding search must be run first.

The pre-launch task builds Dusklight and compiles all checked TAS fixtures.
The selector dispatches named scenarios rather than treating every tape as
interchangeable. Eye Shredder uses the checked Eye runner, fixed-step timing,
the bounded Cursor Breakout model, semantic oracle, trace, isolated state, and
clean live-controller handoff. The smoke scenario uses the generic fixed-step
player. It is retained only for diagnosing early boot and menu input: it sends
delayed Start pulses through the title sequence, selects the freshly-created
first save, and stops in active character-name entry with the default name
`Link`.

While a tape is loaded, Dusklight quarantines host keyboard, mouse, touch, pen,
joystick, and gamepad events from both the game and its UI. Mouse-camera and
gyro polling are also disabled. Window lifecycle events still work, so the
window can be closed normally. This prevents a real controller or an accidental
key press from perturbing playback.

Before launching, the wrapper removes copied surrounding quotes, resolves
relative paths against the repository, and verifies that both files exist. It
prints the exact absolute paths it passes to Dusklight. A missing or stale DVD
prompt falls back to Dusklight's last valid manual selection rather than
silently opening the prelaunch screen.

Every task-launched playback receives a unique writable data root under
`build/automation-state/ephemeral`. That root contains the run's configuration,
memory card, logs, achievements, caches, and controller preferences, and is
removed when the process exits. The normal Dusklight profile and memory card
are never opened for writing. To preserve a run deliberately, invoke the script
with `-StatePath`; that directory is writable and survives process exit:

```powershell
.\tools\glitch-hunting\play-visual-tas.ps1 `
  -TapePath 'build\boot-start-smoke.tape' `
  -StatePath 'build\automation-state\saved-runs\my-run'
```

This is live game playback, not video rendering of a previous run. After a
successful finite visual tape completes, the task intentionally remains active:
Dusklight releases automation ownership and hands control to the live controller
until the window is closed. The Eye runner finalizes and validates its oracle
and trace after process exit.

The named PowerShell dispatcher remains available for checked regression
scenarios such as Eye Shredder. It is no longer exposed as a growing set of VS
Code launch configurations; route exploration belongs in the workbench and
semantic oracle validation belongs in the fixed test selector.

The named dispatcher is also directly callable. Surrounding quotes pasted as
part of the DVD path are accepted:

```powershell
.\tools\glitch-hunting\play-visual-scenario.ps1 `
  -Scenario eye-shredder `
  -DvdPath 'C:\path with spaces\game.iso'
```

The generic launcher remains available for custom and smoke tapes:

```powershell
.\tools\glitch-hunting\play-visual-tas.ps1 `
  -DvdPath 'C:\path with spaces\game.iso' `
  -TapePath 'build\boot-start-smoke.tape'
```

Use `-DryRun` with either visual script to inspect routing or engine arguments
without launching Dusklight.

Direct automated launches are strict: if `--dvd` or `--configured-dvd` cannot
open a valid image, Dusklight exits with an error instead of falling back to the
interactive prelaunch screen. `--automation-data-root` likewise requires an
input tape and an existing directory.

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
known configurations before the tape begins. For a direct launch, create a
stable directory and pass it separately from the disposable automation state:

```powershell
New-Item -ItemType Directory -Force build/renderer-cache | Out-Null
New-Item -ItemType Directory -Force build/automation-state/run-001 | Out-Null
dusklight --dvd game.iso --fixed-step --input-tape run.tape `
  --automation-data-root build/automation-state/run-001 `
  --renderer-cache-root build/renderer-cache
```

## Authoring a custom tape

Copy the smoke `.tas` fixture, define reusable controller states, add exact
frames/repeats/cycles, and compile it:

```powershell
cargo run --manifest-path tools/huntctl/Cargo.toml -- tape compile `
  my-test.tas build/my-test.tape
```

Pass `build/my-test.tape` to the generic visual TAS script. Inspect
the expanded controller frames before playback with:

```powershell
cargo run --manifest-path tools/huntctl/Cargo.toml -- tape inspect `
  build/my-test.tape --frames
```

Visible playback is intentionally paced. For a fast non-rendering replay of
the same tape, run:

```powershell
.\build\windows-clang-debug\dusklight.exe --headless --dvd game.iso `
  --input-tape build/my-test.tape --input-tape-end hold `
  --automation-data-root build/automation-state/my-headless-run `
  --exit-after-tape
```

Create that data directory before a direct launch. The checked PowerShell
launcher does this automatically and is the safer default when fresh-state
cleanup matters.

Custom tapes can be scored against the direct-stage next-map goal without
changing a checked scenario:

```powershell
.\tools\glitch-hunting\evaluate-candidate.ps1 `
  -CandidateId experiment-42 `
  -CandidateTape build\experiment-42.tape
```

The evaluator always launches an isolated writable profile, evaluates native
memory-backed milestones, records a compact diagnostic gameplay trace, and
writes `evaluation.json`, `milestones.json`, and `trace.summary.json` under
`build/test-results/route-search`. A movement sequence that misses the goal is
a successful evaluation with `success: false`; setup, launch, or native-result
failures use `evaluation_status: worker_error`. Trace decoding is diagnostic
and cannot change success or score. This distinction lets an optimizer treat
bad candidates as samples without hiding broken workers.

The checked seed tape starts with 180 neutral frames for direct-stage loading
and the short automatic opening event. Search candidates currently need the
same fixed prefix. Direct stage/save initialization can also be supplied with
the existing `--stage` and `--load-save` CLI options, but those fields are not
yet embedded in a portable tape artifact.
