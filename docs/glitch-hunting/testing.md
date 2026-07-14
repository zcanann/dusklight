# Testing and visual TAS playback

## VS Code test selector

Run **Tasks: Run Test Task** from the command palette and choose
**Glitch Hunt: Run Selected Test**. VS Code presents one fixed selector:

- `all`: native automation, Aurora card/time, Rust tests/lint, and a real
  two-worker smoke run;
- `native`: every game-data-free C++ and Aurora test;
- `input-tape`, `game-clock`, `name-entry`, `name-entry-trace`, `rng`,
  `eye-shredder-oracle`, `aurora-card`, or `aurora-time`: one native suite;
- `eye-shredder`: build and run the real isolated Eye Shredder tape three times
  silently and headlessly against the last configured GCN USA image;
- `intro-first-exit`: run the normal-file first-loading-trigger golf three
  times with semantic gameplay traces;
- `intro-cutscene`: continue the route through the fence into `demo01_04`;
- `rust` or `rust-lint`: the control-plane tests or formatting/Clippy; and
- `worker-smoke` or `pool-smoke`: real native process protocol tests.

The same selector works without VS Code:

```powershell
.\tools\glitch-hunting\run-tests.ps1 -Test all
.\tools\glitch-hunting\run-tests.ps1 -Test input-tape
.\tools\glitch-hunting\run-tests.ps1 -Test pool-smoke
```

The runner configures `windows-clang-debug` with code mods disabled. This keeps
the automation tests usable from ordinary PowerShell rather than requiring a
Visual Studio developer shell.

## Visible TAS playback

For the named Eye Shredder benchmark, run **Tasks: Run Task → Glitch Hunt: Eye
Shredder Visual**. It resolves the last manually selected image, builds the
fixture, runs the memory, renderer, opening-event, and playable-gameplay oracle,
and preserves the result/trace under `build/test-results/eye-shredder`.
Playback owns all four controller ports and consumes only absolute frames. DVD
and memory-card work is dispatched synchronously for fixed-step tape runs; any
deviation from the checked simulation ticks or complete trace hash fails
instead of delaying or retrying input. After the 642-frame tape reaches normal
`F_SP103` gameplay, automation releases the ports and live controller input is
restored. Aurora's XF=12/BP=4 shredded diagnostic remains synthetic;
pixel-perfect console raster corruption is not claimed.

1. Put a supported Twilight Princess disc image somewhere on your machine. A
   repository-local image such as `orig/GZ2E01/GZ2E01.iso` remains ignored by
   Git.
2. Run **Tasks: Run Task** and choose **Glitch Hunt: Play Visual Scenario**.
3. Choose `eye-shredder`, `intro-first-exit`, or `intro-cutscene` from the fixed
   dropdown. Eye Shredder is the default; `boot-start-smoke` is only a short
   boot/menu diagnostic. Leaving the DVD
   prompt blank uses the last image selected through Dusklight's Browse screen.

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
  -TapePath 'build\boot_start_smoke.tape' `
  -StatePath 'build\automation-state\saved-runs\my-run'
```

This is live game playback, not video rendering of a previous run. After a
successful finite visual tape completes, the task intentionally remains active:
Dusklight releases automation ownership and hands control to the live controller
until the window is closed. The Eye runner finalizes and validates its oracle
and trace after process exit.

To attach a debugger and use breakpoints, choose **Glitch Hunt: Eye Shredder
Visual Debug** or **Glitch Hunt: Boot Start Smoke Visual Debug** under **Run and
Debug**, then press F5. They are separate because Eye Shredder requires its
fixed-step shadow-memory and oracle flags; silently launching it as a generic
tape does not reproduce the exploit. Each configuration copies only the last
configured DVD path into a fresh debug data root before launch, then deletes the
root after debugging. They require VS Code's `cppvsdbg` debugger support.

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
  -TapePath 'build\boot_start_smoke.tape'
```

Use `-DryRun` with either visual script to inspect routing or engine arguments
without launching Dusklight.

Direct automated launches are strict: if `--dvd` or `--configured-dvd` cannot
open a valid image, Dusklight exits with an error instead of falling back to the
interactive prelaunch screen. `--automation-data-root` likewise requires an
input tape and an existing directory.

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

The current tape starts at process boot. Direct stage/save initialization can
still be supplied with the existing `--stage` and `--load-save` CLI options,
but those fields are not yet embedded in a portable scenario artifact.
